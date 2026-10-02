//! Credentials for the providers micro talks to.

pub mod anthropic;
pub mod chatgpt;
pub mod codex;
pub mod copilot;
pub mod kimi;
pub mod lockfile;
pub mod oauth;
pub mod openrouter;
pub mod xai;

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
pub const XAI: &str = "xai";
pub const KIMI_CODING: &str = "kimi-coding";

pub const OPENAI_CODEX: &str = "openai-codex";

/// One service micro can authenticate.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderEntry {
    /// The canonical id: the key its credential is stored under, and the name a UI hands back.
    pub id: String,
    /// The name to show a person.
    pub name: String,
    /// Environment variables that supply its key, in the order they are tried.
    pub env: Vec<String>,
    /// What to call the credential when asking for it.
    pub key: String,
}

/// Every provider micro can authenticate, generated alongside the model catalog so the two always
/// name the same services.
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

/// Fold a name onto the id everything else uses.
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
    /// The user authorizes micro in a browser, through a browser redirect or a device code.
    OAuth,
}

/// The login a provider presents first: an account sign-in for a provider that has nothing else,
/// a key otherwise.
pub fn auth_method(provider: &str) -> AuthMethod {
    match oauth_login(provider) {
        Some(login) if !login.api_key => AuthMethod::OAuth,
        _ => AuthMethod::ApiKey,
    }
}

/// Paste a key.
pub const METHOD_API_KEY: &str = "api_key";
/// Sign in with the provider's account, by whichever method it has.
pub const METHOD_OAUTH: &str = "oauth";
/// Sign in through a browser that returns to a loopback listener.
pub const METHOD_BROWSER: &str = "browser";
/// Sign in through a browser on another machine, pasting the code it shows.
pub const METHOD_COPY_CODE: &str = "copy_code";
/// Sign in by typing a code into a page opened anywhere.
pub const METHOD_DEVICE_CODE: &str = "device_code";

/// How a provider's account sign-in is described and offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OAuthLogin {
    /// The name of the account, such as `Anthropic (Claude Pro/Max)`.
    pub name: &'static str,
    /// What the option that starts it says.
    pub label: &'static str,
    /// Whether the account is a paid subscription rather than an account of another kind.
    pub subscription: bool,
    /// The ways in, as `(id, label)`, when there is more than one.
    pub methods: &'static [(&'static str, &'static str)],
    /// Whether the provider also takes an API key.
    pub api_key: bool,
}

const ACCOUNT_LABEL: &str = "Sign in with an account";
const API_KEY_LABEL: &str = "Sign in with an API key";
const BROWSER_METHOD: (&str, &str) = (METHOD_BROWSER, "Browser login (default)");

/// The account sign-in a provider offers, if any.
pub fn oauth_login(provider: &str) -> Option<OAuthLogin> {
    let login = match canonical_provider(provider) {
        ANTHROPIC => OAuthLogin {
            name: "Anthropic (Claude Pro/Max)",
            label: ACCOUNT_LABEL,
            subscription: true,
            methods: &[
                BROWSER_METHOD,
                (METHOD_COPY_CODE, "Copy code login (headless)"),
            ],
            api_key: true,
        },
        OPENAI => OAuthLogin {
            name: "OpenAI (ChatGPT subscription)",
            label: "Sign in with ChatGPT",
            subscription: true,
            methods: &[],
            api_key: true,
        },
        OPENAI_CODEX => OAuthLogin {
            name: "OpenAI (ChatGPT Plus/Pro)",
            label: ACCOUNT_LABEL,
            subscription: true,
            methods: &[
                BROWSER_METHOD,
                (METHOD_DEVICE_CODE, "Device code login (headless)"),
            ],
            api_key: false,
        },
        GITHUB_COPILOT => OAuthLogin {
            name: "GitHub Copilot",
            label: ACCOUNT_LABEL,
            subscription: true,
            methods: &[],
            api_key: false,
        },
        OPENROUTER => OAuthLogin {
            name: "OpenRouter OAuth",
            label: "Sign in with OpenRouter",
            subscription: false,
            methods: &[],
            api_key: true,
        },
        XAI => OAuthLogin {
            name: "xAI (Grok/X subscription)",
            label: "Sign in with SuperGrok or X Premium",
            subscription: true,
            methods: &[],
            api_key: true,
        },
        KIMI_CODING => OAuthLogin {
            name: "Kimi Code (subscription)",
            label: "Sign in with Kimi Code",
            subscription: true,
            methods: &[],
            api_key: true,
        },
        _ => return None,
    };
    Some(login)
}

/// What a credential of this kind is called: `API key`, or `subscription` or `account` for a
/// sign-in, depending on whether the provider's account is a subscription.
pub fn credential_kind(provider: &str, oauth: bool) -> &'static str {
    match (oauth, oauth_login(provider)) {
        (false, _) => "API key",
        (true, Some(login)) if !login.subscription => "account",
        (true, _) => "subscription",
    }
}

/// What a login needs from the application that starts it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginOptions {
    /// This installation's stable UUID, which Sign in with ChatGPT registers the host under.
    pub device_id: Option<String>,
}

/// Where sign-ins and refreshes are sent. Every field defaults to the provider's real service.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub anthropic_token: String,
    /// Where workload identity federation exchanges an identity token.
    pub anthropic_api: String,
    pub codex: codex::Endpoints,
    pub chatgpt_token: String,
    pub openrouter_token: String,
    pub xai: xai::Endpoints,
    /// The Kimi authorization host, or the one the environment names.
    pub kimi_host: Option<String>,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            anthropic_token: anthropic::TOKEN_URL.to_string(),
            anthropic_api: anthropic::API_BASE_URL.to_string(),
            codex: codex::Endpoints::default(),
            chatgpt_token: chatgpt::TOKEN_URL.to_string(),
            openrouter_token: openrouter::TOKEN_URL.to_string(),
            xai: xai::Endpoints::default(),
            kimi_host: None,
        }
    }
}

impl Endpoints {
    fn kimi_host(&self) -> String {
        self.kimi_host
            .clone()
            .unwrap_or_else(|| kimi::oauth_host(|name| std::env::var(name).ok()))
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

    #[error("device authorization failed: {0}")]
    DeviceFlow(String),

    #[error("Copilot token exchange failed: {0}")]
    TokenExchange(String),

    #[error("{0}")]
    OAuth(String),

    #[error("Anthropic workload identity federation: {0}")]
    Federation(String),

    #[error("login cancelled")]
    Cancelled,

    #[error("`{provider}` has no login method `{method}`")]
    UnknownMethod { provider: String, method: String },

    #[error("`{provider}` is not signed in with an account, so it has no bearer token")]
    NotOAuth { provider: String },
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

/// An OAuth credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCredential {
    pub access_token: String,
    pub refresh_token: String,
    /// Milliseconds since the Unix epoch.
    pub expires: i64,
    /// The client the provider issued for this sign-in, which refreshes must be made as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
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
    /// Prompt for a key, then hand it to [`AuthStore::store_api_key`].
    ApiKey {
        provider: String,
        env_names: Vec<String>,
    },
    /// Ask which way in, then begin again with [`AuthStore::begin_login_with`] and the chosen id.
    Choose {
        provider: String,
        title: String,
        options: Vec<LoginOption>,
    },
    /// Show the URL and code, then await [`AuthStore::complete_device_login`].
    DeviceCode(PendingDeviceLogin),
    /// Open the URL, offer to take what the browser shows, and await
    /// [`AuthStore::complete_browser_login`].
    Browser(PendingBrowserLogin),
}

/// One way into a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOption {
    pub id: String,
    pub label: String,
}

/// Which service a device code is redeemed with.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeviceFlow {
    Copilot,
    Codex,
    Xai,
    Kimi { host: String },
}

/// A device authorization waiting to be redeemed.
pub struct PendingDeviceLogin {
    pub provider: String,
    pub authorization: oauth::DeviceAuthorization,
    flow: DeviceFlow,
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

/// How a browser sign-in's code is redeemed, and what it was started with.
enum BrowserExchange {
    Anthropic {
        pkce: oauth::Pkce,
        redirect_uri: String,
    },
    Codex {
        pkce: oauth::Pkce,
        state: String,
    },
    ChatGpt {
        pkce: oauth::Pkce,
        state: String,
    },
    OpenRouter {
        pkce: oauth::Pkce,
    },
}

/// A browser sign-in waiting for the browser to come back, or for the user to paste what it shows.
pub struct PendingBrowserLogin {
    pub provider: String,
    url: String,
    instructions: String,
    prompt: String,
    placeholder: String,
    note: Option<String>,
    callback: Option<oauth::CallbackServer>,
    exchange: BrowserExchange,
}

impl PendingBrowserLogin {
    /// The page the user signs in on.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// What to tell the user about finishing the sign-in.
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// What to ask when offering to take a pasted code or redirect URL.
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// An example of what may be pasted.
    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    /// Something the user should know before signing in, such as a callback that could not listen.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Whether the browser can finish the sign-in without anything pasted.
    pub fn listens(&self) -> bool {
        self.callback.is_some()
    }
}

impl std::fmt::Debug for PendingBrowserLogin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingBrowserLogin")
            .field("provider", &self.provider)
            .field("url", &self.url)
            .field("listens", &self.listens())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSource {
    /// The credential file.
    Stored,
    /// An environment variable, which micro reads but never writes.
    Environment {
        variable: String,
    },
    /// Anthropic workload identity federation, configured in the environment.
    Federation,
    Missing,
}

/// One provider's standing, for a UI to render without touching the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatus {
    pub provider: String,
    pub method: AuthMethod,
    pub source: CredentialSource,
    /// When the stored token lapses, in milliseconds since the Unix epoch.
    pub expires: Option<i64>,
    /// The stored token is past its expiry and will be exchanged on the next request.
    pub needs_refresh: bool,
}

impl ProviderStatus {
    pub fn is_authenticated(&self) -> bool {
        !matches!(self.source, CredentialSource::Missing)
    }
}

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
    endpoints: Endpoints,
    federation: anthropic::FederationCache,
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
            endpoints: Endpoints::default(),
            federation: anthropic::FederationCache::default(),
        })
    }

    /// Send sign-ins and refreshes somewhere other than the providers' own services.
    pub fn with_endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = endpoints;
        self
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

    /// A credential ready to send: stored if present, otherwise from the environment, refreshed
    /// first if the provider's tokens expire.
    pub async fn resolve(&self, provider: &str) -> Result<Credential> {
        self.resolve_valid_for(provider, 0).await
    }

    /// A credential that stays valid for at least `min_validity_ms`, refreshing a stored OAuth
    /// token that would lapse sooner.
    pub async fn resolve_valid_for(
        &self,
        provider: &str,
        min_validity_ms: i64,
    ) -> Result<Credential> {
        let provider = canonical_provider(provider);

        if let Some(stored) = self.get(provider) {
            return self.prepare(provider, stored, true, min_validity_ms).await;
        }

        if let Some(value) = env_value(provider, |name| std::env::var(name).ok()) {
            return self
                .prepare(provider, from_env(provider, value), false, min_validity_ms)
                .await;
        }

        if provider == ANTHROPIC {
            if let Some(federation) =
                anthropic::Federation::from_env(|name| std::env::var(name).ok())
            {
                let token = self
                    .federation
                    .token(&self.http, &federation, &self.endpoints.anthropic_api)
                    .await?;
                return Ok(Credential::OAuth(OAuthCredential {
                    access_token: token.access_token,
                    refresh_token: String::new(),
                    expires: token.expires,
                    client_id: None,
                }));
            }
        }

        Err(AuthError::Missing {
            provider: provider.to_string(),
            env: env_names(provider).join(", "),
        })
    }

    /// Force a refresh of a stored OAuth credential, whatever its recorded expiry.
    pub async fn refresh(&self, provider: &str) -> Result<Credential> {
        let provider = canonical_provider(provider);
        let Some(Credential::OAuth(_)) = self.get(provider) else {
            return Err(AuthError::NoRefresh {
                provider: provider.to_string(),
            });
        };
        self.refresh_stored(provider, None).await
    }

    /// Exchange a stored refresh token while holding the file against every other process, so a
    /// token the provider rotates is spent once. A process that waited finds the credential another
    /// one already refreshed, and uses it when it is fresh enough.
    async fn refresh_stored(
        &self,
        provider: &str,
        min_validity_ms: Option<i64>,
    ) -> Result<Credential> {
        let path = self.path.clone();
        let held = tokio::task::spawn_blocking(move || lockfile::FileLock::acquire(&path))
            .await
            .map_err(|error| AuthError::Storage {
                path: self.path.display().to_string(),
                message: error.to_string(),
            })?
            .map_err(|error| storage_error(&self.path, error))?;

        let mut latest = load(&self.path)?;
        let current = match latest.get(provider) {
            Some(Credential::OAuth(oauth)) => oauth.clone(),
            _ => {
                return Err(AuthError::NoRefresh {
                    provider: provider.to_string(),
                })
            }
        };
        if let Some(min_validity_ms) = min_validity_ms {
            if !needs_refresh(provider, &current, now_ms(), min_validity_ms) {
                return Ok(Credential::OAuth(current));
            }
        }

        let refreshed = Credential::OAuth(self.refresh_oauth(provider, &current).await?);
        latest.insert(provider.to_string(), refreshed.clone());
        save(&self.path, &latest)?;
        drop(held);

        let mut cache = self.lock();
        cache.credentials = latest;
        cache.revision = revision_of(&self.path);
        Ok(refreshed)
    }

    /// Begin an interactive login, asking which way in when the provider has more than one.
    pub async fn begin_login(&self, provider: &str) -> Result<LoginFlow> {
        self.begin_login_with(provider, None, &LoginOptions::default())
            .await
    }

    /// Begin an interactive login by the method with this id, or ask for one when `method` is
    /// `None` and there is a choice to make.
    pub async fn begin_login_with(
        &self,
        provider: &str,
        method: Option<&str>,
        options: &LoginOptions,
    ) -> Result<LoginFlow> {
        let provider = canonical_provider(provider).to_string();
        let login = oauth_login(&provider);
        let unknown = |method: &str| AuthError::UnknownMethod {
            provider: provider.clone(),
            method: method.to_string(),
        };

        let method = match (method, login) {
            (Some(METHOD_API_KEY), Some(login)) if !login.api_key => {
                return Err(unknown(METHOD_API_KEY))
            }
            (Some(METHOD_API_KEY), _) | (None, None) => {
                return Ok(LoginFlow::ApiKey {
                    env_names: env_names(&provider),
                    provider,
                })
            }
            (Some(method), None) => return Err(unknown(method)),
            (None, Some(login)) if login.api_key => {
                return Ok(LoginFlow::Choose {
                    title: format!(
                        "Select authentication method for {}:",
                        provider_entry(&provider)
                            .map(|entry| entry.name.as_str())
                            .unwrap_or(&provider)
                    ),
                    options: vec![
                        LoginOption {
                            id: METHOD_OAUTH.into(),
                            label: login.label.into(),
                        },
                        LoginOption {
                            id: METHOD_API_KEY.into(),
                            label: API_KEY_LABEL.into(),
                        },
                    ],
                    provider,
                })
            }
            (None | Some(METHOD_OAUTH), Some(login)) if !login.methods.is_empty() => {
                return Ok(LoginFlow::Choose {
                    title: format!("Select {} login method:", login.name),
                    options: login
                        .methods
                        .iter()
                        .map(|(id, label)| LoginOption {
                            id: id.to_string(),
                            label: label.to_string(),
                        })
                        .collect(),
                    provider,
                })
            }
            (None | Some(METHOD_OAUTH), Some(_)) => None,
            (Some(method), Some(login)) => {
                if !login.methods.iter().any(|(id, _)| *id == method) {
                    return Err(unknown(method));
                }
                Some(method)
            }
        };

        match (provider.as_str(), method) {
            (GITHUB_COPILOT, _) => Ok(LoginFlow::DeviceCode(PendingDeviceLogin {
                authorization: copilot::start_device_flow(&self.http).await?,
                flow: DeviceFlow::Copilot,
                provider,
            })),
            (ANTHROPIC, Some(METHOD_COPY_CODE)) => Ok(LoginFlow::Browser(anthropic_copy_code())),
            (ANTHROPIC, _) => Ok(LoginFlow::Browser(anthropic_browser().await)),
            (OPENAI_CODEX, Some(METHOD_DEVICE_CODE)) => {
                Ok(LoginFlow::DeviceCode(PendingDeviceLogin {
                    authorization: codex::start_device_flow(&self.http, &self.endpoints.codex)
                        .await?,
                    flow: DeviceFlow::Codex,
                    provider,
                }))
            }
            (OPENAI_CODEX, _) => Ok(LoginFlow::Browser(codex_browser().await)),
            (OPENAI, _) => Ok(LoginFlow::Browser(chatgpt_browser(options).await?)),
            (OPENROUTER, _) => Ok(LoginFlow::Browser(openrouter_browser().await?)),
            (XAI, _) => Ok(LoginFlow::DeviceCode(PendingDeviceLogin {
                authorization: xai::start_device_flow(&self.http, &self.endpoints.xai).await?,
                flow: DeviceFlow::Xai,
                provider,
            })),
            (KIMI_CODING, _) => {
                let host = self.endpoints.kimi_host();
                Ok(LoginFlow::DeviceCode(PendingDeviceLogin {
                    authorization: kimi::start_device_flow(&self.http, &host).await?,
                    flow: DeviceFlow::Kimi { host },
                    provider,
                }))
            }
            (_, method) => Err(unknown(method.unwrap_or(METHOD_OAUTH))),
        }
    }

    /// Wait for the user to finish authorizing in the browser, then store the credential.
    pub async fn complete_device_login(&self, pending: &PendingDeviceLogin) -> Result<Credential> {
        let authorization = &pending.authorization;
        let credential = Credential::OAuth(match &pending.flow {
            DeviceFlow::Copilot => copilot::poll_for_token(&self.http, authorization).await?,
            DeviceFlow::Codex => {
                codex::poll_device_flow(&self.http, &self.endpoints.codex, authorization).await?
            }
            DeviceFlow::Xai => {
                xai::poll_device_flow(&self.http, &self.endpoints.xai, authorization).await?
            }
            DeviceFlow::Kimi { host } => {
                kimi::poll_device_flow(&self.http, host, authorization).await?
            }
        });
        self.set(&pending.provider, credential.clone())?;
        Ok(credential)
    }

    /// Wait for the browser to return, or for `manual` to yield what the user pasted, then redeem
    /// the code and store the credential. `manual` yields nothing when the user dismissed the
    /// prompt, which leaves only the browser to finish the sign-in.
    pub async fn complete_browser_login<M>(
        &self,
        pending: &PendingBrowserLogin,
        manual: M,
    ) -> Result<Credential>
    where
        M: std::future::Future<Output = Option<String>>,
    {
        let returned = oauth::callback_or_manual(pending.callback.as_ref(), manual).await?;
        let credential = Credential::OAuth(self.redeem(&pending.exchange, returned).await?);
        self.set(&pending.provider, credential.clone())?;
        Ok(credential)
    }

    async fn redeem(
        &self,
        exchange: &BrowserExchange,
        returned: oauth::CallbackOrManual,
    ) -> Result<OAuthCredential> {
        use oauth::CallbackOrManual::Callback;
        use oauth::CallbackOrManual::Manual;
        let missing = || AuthError::OAuth("missing authorization code".into());

        match exchange {
            BrowserExchange::Anthropic { pkce, redirect_uri } => {
                let (code, state) = match returned {
                    Callback(params) => (params.get("code").cloned(), pkce.verifier.clone()),
                    Manual(input) => {
                        let parsed = oauth::parse_authorization_input(&input);
                        if parsed
                            .state
                            .as_ref()
                            .is_some_and(|state| *state != pkce.verifier)
                        {
                            return Err(AuthError::OAuth("OAuth state mismatch".into()));
                        }
                        (
                            parsed.code,
                            parsed.state.unwrap_or_else(|| pkce.verifier.clone()),
                        )
                    }
                };
                let code = code.ok_or_else(missing)?;
                anthropic::exchange_code(
                    &self.http,
                    &self.endpoints.anthropic_token,
                    &code,
                    &state,
                    &pkce.verifier,
                    redirect_uri,
                )
                .await
            }
            BrowserExchange::Codex { pkce, state } => {
                let code = match returned {
                    Callback(params) => params.get("code").cloned(),
                    Manual(input) => {
                        let parsed = oauth::parse_authorization_input(&input);
                        if parsed.state.as_ref().is_some_and(|sent| sent != state) {
                            return Err(AuthError::OAuth("OAuth state mismatch".into()));
                        }
                        parsed.code
                    }
                };
                let code = code.ok_or_else(missing)?;
                codex::exchange_code(
                    &self.http,
                    &self.endpoints.codex,
                    &code,
                    &pkce.verifier,
                    codex::redirect_uri(),
                )
                .await
            }
            BrowserExchange::ChatGpt { pkce, state } => {
                let authorization = match returned {
                    Callback(params) => chatgpt::authorization_from_callback(&params, state)?,
                    Manual(input) => chatgpt::authorization_from_pasted(&input, state)?,
                };
                chatgpt::exchange_code(
                    &self.http,
                    &self.endpoints.chatgpt_token,
                    &authorization,
                    &pkce.verifier,
                )
                .await
            }
            BrowserExchange::OpenRouter { pkce } => {
                let code = match returned {
                    Callback(params) => params.get("code").cloned(),
                    Manual(input) => openrouter::code_from_pasted(&input),
                };
                let code = code.ok_or_else(missing)?;
                openrouter::exchange_code(
                    &self.http,
                    &self.endpoints.openrouter_token,
                    &code,
                    &pkce.verifier,
                )
                .await
            }
        }
    }

    /// Store a key the user pasted.
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

    /// Forget a provider's stored credential.
    pub fn logout(&self, provider: &str) -> Result<()> {
        self.remove(provider)
    }

    /// Where every provider stands, for a UI to render.
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

        let (method, source, expires, needs_refresh) = match self.get(&provider) {
            Some(Credential::OAuth(oauth)) => (
                AuthMethod::OAuth,
                CredentialSource::Stored,
                (oauth.expires > 0 && oauth.expires < openrouter::NEVER_EXPIRES)
                    .then_some(oauth.expires),
                needs_refresh(&provider, &oauth, now_ms(), 0),
            ),
            Some(Credential::ApiKey { .. }) => {
                (AuthMethod::ApiKey, CredentialSource::Stored, None, false)
            }
            None => {
                let method = auth_method(&provider);
                let variable = env_names(&provider)
                    .into_iter()
                    .find(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()));
                let federated = provider == ANTHROPIC
                    && anthropic::Federation::from_env(|name| std::env::var(name).ok()).is_some();
                match (variable, federated) {
                    (Some(variable), _) => (
                        method,
                        CredentialSource::Environment { variable },
                        None,
                        false,
                    ),
                    (None, true) => (method, CredentialSource::Federation, None, false),
                    (None, false) => (method, CredentialSource::Missing, None, false),
                }
            }
        };

        ProviderStatus {
            provider,
            method,
            source,
            expires,
            needs_refresh,
        }
    }

    /// Only credentials that came from the file are written back; an environment token is the
    /// user's to manage.
    async fn prepare(
        &self,
        provider: &str,
        credential: Credential,
        persist: bool,
        min_validity_ms: i64,
    ) -> Result<Credential> {
        match credential {
            Credential::ApiKey { key } => Ok(Credential::ApiKey {
                key: expand(&key, |name| std::env::var(name).ok()),
            }),
            Credential::OAuth(oauth)
                if needs_refresh(provider, &oauth, now_ms(), min_validity_ms) =>
            {
                match persist {
                    true => self.refresh_stored(provider, Some(min_validity_ms)).await,
                    false => Ok(Credential::OAuth(
                        self.refresh_oauth(provider, &oauth).await?,
                    )),
                }
            }
            other => Ok(other),
        }
    }

    async fn refresh_oauth(
        &self,
        provider: &str,
        credential: &OAuthCredential,
    ) -> Result<OAuthCredential> {
        let http = &self.http;
        let endpoints = &self.endpoints;
        match provider {
            GITHUB_COPILOT => copilot::exchange_token(http, &credential.refresh_token).await,
            ANTHROPIC => {
                anthropic::refresh(http, &endpoints.anthropic_token, &credential.refresh_token)
                    .await
            }
            OPENAI_CODEX => codex::refresh(http, &endpoints.codex, &credential.refresh_token).await,
            OPENAI => chatgpt::refresh(http, &endpoints.chatgpt_token, credential).await,
            XAI => xai::refresh(http, &endpoints.xai, &credential.refresh_token).await,
            KIMI_CODING => {
                kimi::refresh(http, &endpoints.kimi_host(), &credential.refresh_token).await
            }
            _ => Err(AuthError::NoRefresh {
                provider: provider.to_string(),
            }),
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

/// Anthropic's browser sign-in, with a loopback listener when its port is free.
async fn anthropic_browser() -> PendingBrowserLogin {
    let pkce = oauth::Pkce::generate();
    let redirect_uri = anthropic::browser_redirect_uri();
    let callback = anthropic::start_callback(&pkce).await;
    PendingBrowserLogin {
        provider: ANTHROPIC.into(),
        url: anthropic::authorize_url(&pkce, &redirect_uri),
        instructions: "Complete login in your browser. If the browser is on another machine, \
                       paste the final redirect URL here."
            .into(),
        prompt: "Complete login in your browser, or paste the authorization code / redirect URL \
                 here:"
            .into(),
        placeholder: redirect_uri.clone(),
        note: None,
        callback,
        exchange: BrowserExchange::Anthropic { pkce, redirect_uri },
    }
}

/// Anthropic's sign-in for a browser on another machine: it shows a code to paste back.
fn anthropic_copy_code() -> PendingBrowserLogin {
    let pkce = oauth::Pkce::generate();
    let redirect_uri = anthropic::copy_code_redirect_uri().to_string();
    PendingBrowserLogin {
        provider: ANTHROPIC.into(),
        url: anthropic::authorize_url(&pkce, &redirect_uri),
        instructions: "Complete login in your browser, then copy the code Anthropic shows and \
                       paste it here."
            .into(),
        prompt: "Paste the code Anthropic shows after you sign in:".into(),
        placeholder: "code#state".into(),
        note: None,
        callback: None,
        exchange: BrowserExchange::Anthropic { pkce, redirect_uri },
    }
}

async fn codex_browser() -> PendingBrowserLogin {
    let pkce = oauth::Pkce::generate();
    let state = oauth::random_hex(16);
    let callback = codex::start_callback(&state).await;
    PendingBrowserLogin {
        provider: OPENAI_CODEX.into(),
        url: codex::authorize_url(&pkce, &state),
        instructions: "A browser window should open. Complete login to finish.".into(),
        prompt: "Complete login in your browser, or paste the authorization code / redirect URL \
                 here:"
            .into(),
        placeholder: codex::redirect_uri().into(),
        note: None,
        callback,
        exchange: BrowserExchange::Codex { pkce, state },
    }
}

async fn chatgpt_browser(options: &LoginOptions) -> Result<PendingBrowserLogin> {
    let host_id = chatgpt::agent_host_id(options.device_id.as_deref().unwrap_or_default())?;
    let pkce = oauth::Pkce::generate();
    let state = oauth::random_base64url(32);
    let nonce = oauth::random_base64url(32);
    let (callback, note) = match chatgpt::start_callback(&state).await {
        Ok(callback) => (Some(callback), None),
        Err(error) => (
            None,
            Some(format!(
                "Could not listen on {}; paste the final redirect URL to continue. {error}",
                chatgpt::redirect_uri()
            )),
        ),
    };
    Ok(PendingBrowserLogin {
        provider: OPENAI.into(),
        url: chatgpt::authorize_url(&pkce, &state, &nonce, &host_id),
        instructions: "Complete sign-in in your browser. If the callback does not complete, \
                       paste the final redirect URL here."
            .into(),
        prompt: "Complete login in your browser, or paste the final redirect URL here:".into(),
        placeholder: chatgpt::redirect_uri().into(),
        note,
        callback,
        exchange: BrowserExchange::ChatGpt { pkce, state },
    })
}

async fn openrouter_browser() -> Result<PendingBrowserLogin> {
    let pkce = oauth::Pkce::generate();
    let callback = openrouter::start_callback().await.map_err(|error| {
        AuthError::OAuth(format!(
            "cannot listen for the OpenRouter callback: {error}"
        ))
    })?;
    let redirect_uri = callback.redirect_uri().to_string();
    Ok(PendingBrowserLogin {
        provider: OPENROUTER.into(),
        url: openrouter::authorize_url(&pkce, &redirect_uri),
        instructions: "Complete sign-in in your browser. If the browser is on another machine, \
                       paste the final redirect URL here."
            .into(),
        prompt: "Complete sign-in in your browser, or paste the authorization code / redirect \
                 URL here:"
            .into(),
        placeholder: redirect_uri.clone(),
        note: Some(format!(
            "Listening for the OpenRouter callback on {redirect_uri}"
        )),
        callback: Some(callback),
        exchange: BrowserExchange::OpenRouter { pkce },
    })
}

/// Whether an OAuth credential must be exchanged before it can be used for `min_validity_ms`.
fn needs_refresh(
    provider: &str,
    credential: &OAuthCredential,
    now: i64,
    min_validity_ms: i64,
) -> bool {
    let horizon = now + EXPIRY_SKEW_MS.max(min_validity_ms);
    match provider {
        GITHUB_COPILOT => credential.expires <= horizon,
        _ => credential.expires > 0 && credential.expires <= horizon,
    }
}

/// A Copilot token found in the environment is a GitHub OAuth token.
fn from_env(provider: &str, value: String) -> Credential {
    match provider {
        GITHUB_COPILOT => Credential::OAuth(OAuthCredential {
            access_token: String::new(),
            refresh_token: value,
            expires: 0,
            client_id: None,
        }),
        _ => Credential::ApiKey { key: value },
    }
}

/// Environment variables to try for a provider, in order.
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

/// A stored key may point at the environment as `$VAR`.
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

/// Read the store.
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

/// Write the store through a temporary file created owner-only.
fn save(path: &Path, credentials: &BTreeMap<String, Credential>) -> Result<()> {
    let contents =
        serde_json::to_string_pretty(credentials).map_err(|error| AuthError::Storage {
            path: path.display().to_string(),
            message: format!("could not be encoded: {error}"),
        })?;

    write_private(path, contents.as_bytes()).map_err(|error| storage_error(path, error))
}

/// Replace a file that holds secrets: written through a temporary file only its owner can read,
/// in a directory only its owner can list, then moved into place.
pub fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)?;
    restrict_directory(directory);

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| FILE_NAME.to_string());
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&temporary);

    let mut file = create_owner_only(&temporary)?;
    file.write_all(contents).and_then(|()| file.sync_all())?;
    drop(file);

    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
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
    use crate::oauth::testing::TestServer;
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
            client_id: None,
        })
    }

    /// Two processes writing different providers both keep their work.
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
            "credential store should reload",
        );
    }

    /// A credential is something the user put there, so it travels with the settings and not with
    /// what micro produced.
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
            client_id: None,
        };
        assert!(needs_refresh(GITHUB_COPILOT, &stale, now, 0));
        assert!(!needs_refresh(ANTHROPIC, &stale, now, 0));
    }

    #[test]
    fn a_token_is_refreshed_once_it_is_inside_the_skew_window() {
        let now = 1_000_000;
        let expiring = |expires| OAuthCredential {
            access_token: "token".into(),
            refresh_token: "gho".into(),
            expires,
            client_id: None,
        };

        assert!(needs_refresh(GITHUB_COPILOT, &expiring(now - 1), now, 0));
        assert!(needs_refresh(
            GITHUB_COPILOT,
            &expiring(now + EXPIRY_SKEW_MS - 1),
            now,
            0
        ));
        assert!(!needs_refresh(
            GITHUB_COPILOT,
            &expiring(now + EXPIRY_SKEW_MS + 1),
            now,
            0
        ));
        assert!(!needs_refresh(
            ANTHROPIC,
            &expiring(now + EXPIRY_SKEW_MS + 1),
            now,
            0
        ));
    }

    #[test]
    fn each_provider_has_its_conventional_environment_variables() {
        assert_eq!(env_names(OPENROUTER), vec!["OPENROUTER_API_KEY"]);
        assert_eq!(env_names(GOOGLE), vec!["GEMINI_API_KEY"]);
        assert_eq!(env_names(GITHUB_COPILOT), vec!["COPILOT_GITHUB_TOKEN"]);

        assert_eq!(
            env_names(ANTHROPIC),
            vec![
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_OAUTH_TOKEN",
                "ANTHROPIC_API_KEY"
            ]
        );

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
    fn a_provider_without_keys_logs_in_through_a_browser_first() {
        assert_eq!(auth_method(GITHUB_COPILOT), AuthMethod::OAuth);
        assert_eq!(auth_method("copilot"), AuthMethod::OAuth);
        assert_eq!(auth_method("codex"), AuthMethod::OAuth);
        for provider in [ANTHROPIC, OPENROUTER, GOOGLE, OPENAI, XAI, KIMI_CODING] {
            assert_eq!(auth_method(provider), AuthMethod::ApiKey);
        }
    }

    #[test]
    fn a_sign_in_is_called_a_subscription_unless_the_account_is_not_one() {
        assert_eq!(credential_kind(ANTHROPIC, true), "subscription");
        assert_eq!(credential_kind(XAI, true), "subscription");
        assert_eq!(credential_kind(OPENROUTER, true), "account");
        assert_eq!(credential_kind(OPENROUTER, false), "API key");
        assert_eq!(credential_kind("a-proxy", true), "subscription");
    }

    #[tokio::test]
    async fn a_provider_with_a_key_and_an_account_asks_which() {
        let store = AuthStore::open_at(scratch("choose").join("auth.json")).unwrap();
        let LoginFlow::Choose {
            provider, options, ..
        } = store.begin_login("claude").await.unwrap()
        else {
            panic!("expected a choice");
        };
        assert_eq!(provider, ANTHROPIC);
        let ids: Vec<&str> = options.iter().map(|option| option.id.as_str()).collect();
        assert_eq!(ids, vec![METHOD_OAUTH, METHOD_API_KEY]);

        let LoginFlow::Choose { options, .. } = store
            .begin_login_with(ANTHROPIC, Some(METHOD_OAUTH), &LoginOptions::default())
            .await
            .unwrap()
        else {
            panic!("expected the sign-in methods");
        };
        let ids: Vec<&str> = options.iter().map(|option| option.id.as_str()).collect();
        assert_eq!(ids, vec![METHOD_BROWSER, METHOD_COPY_CODE]);

        let LoginFlow::Choose { options, .. } = store.begin_login("codex").await.unwrap() else {
            panic!("codex has two sign-in methods and no key");
        };
        assert_eq!(options.len(), 2);

        let error = store
            .begin_login_with(OPENAI_CODEX, Some(METHOD_API_KEY), &LoginOptions::default())
            .await
            .err()
            .expect("codex takes no key");
        assert!(matches!(error, AuthError::UnknownMethod { .. }), "{error}");
    }

    #[tokio::test]
    async fn sign_in_with_chatgpt_needs_the_device_id() {
        let store = AuthStore::open_at(scratch("chatgpt-device").join("auth.json")).unwrap();
        let error = store
            .begin_login_with(OPENAI, Some(METHOD_OAUTH), &LoginOptions::default())
            .await
            .err()
            .expect("no device id");
        assert!(error.to_string().contains("device id"), "{error}");
    }

    #[tokio::test]
    async fn the_copy_code_login_redeems_the_pasted_code_and_stores_the_tokens() {
        let server = TestServer::start(vec![(
            200,
            r#"{"access_token":"sk-ant-oat01-x","refresh_token":"sk-ant-ort01-y","expires_in":28800}"#
                .into(),
        )])
        .await;
        let store = AuthStore::open_at(scratch("copy-code").join("auth.json"))
            .unwrap()
            .with_endpoints(Endpoints {
                anthropic_token: server.url("/v1/oauth/token"),
                ..Endpoints::default()
            });

        let LoginFlow::Browser(pending) = store
            .begin_login_with(ANTHROPIC, Some(METHOD_COPY_CODE), &LoginOptions::default())
            .await
            .unwrap()
        else {
            panic!("expected a browser login");
        };
        assert!(!pending.listens(), "the browser is elsewhere");
        let url = reqwest::Url::parse(pending.url()).unwrap();
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .to_string();

        let wrong = store
            .complete_browser_login(&pending, async { Some("code#another-state".to_string()) })
            .await
            .unwrap_err();
        assert!(wrong.to_string().contains("state mismatch"), "{wrong}");

        let pasted = format!("the-code#{state}");
        store
            .complete_browser_login(&pending, async move { Some(pasted) })
            .await
            .unwrap();

        assert_eq!(store.get(ANTHROPIC).unwrap().token(), "sk-ant-oat01-x");
        let sent = server.requests()[0].json();
        assert_eq!(sent["code"], "the-code");
        assert_eq!(sent["redirect_uri"], anthropic::copy_code_redirect_uri());
        assert_eq!(store.status_of(ANTHROPIC).method, AuthMethod::OAuth);
    }

    #[tokio::test]
    async fn an_expired_stored_token_is_refreshed_and_written_back() {
        let server = TestServer::start(vec![(
            200,
            r#"{"access_token":"sk-ant-oat01-new","refresh_token":"rotated","expires_in":3600}"#
                .into(),
        )])
        .await;
        let path = scratch("refresh").join("auth.json");
        let store = AuthStore::open_at(&path)
            .unwrap()
            .with_endpoints(Endpoints {
                anthropic_token: server.url("/v1/oauth/token"),
                ..Endpoints::default()
            });
        store.set(ANTHROPIC, oauth("sk-ant-oat01-old", 1)).unwrap();

        let resolved = store.resolve(ANTHROPIC).await.unwrap();
        assert_eq!(resolved.token(), "sk-ant-oat01-new");
        assert_eq!(server.requests()[0].json()["refresh_token"], "gho_github");

        let Some(Credential::OAuth(written)) = AuthStore::open_at(&path).unwrap().get(ANTHROPIC)
        else {
            panic!("the refreshed credential was written");
        };
        assert_eq!(written.refresh_token, "rotated");
    }

    #[tokio::test]
    async fn a_token_valid_long_enough_is_not_refreshed_but_a_shorter_one_is() {
        let server = TestServer::start(vec![(
            200,
            r#"{"access_token":"fresh","refresh_token":"r2","expires_in":7200}"#.into(),
        )])
        .await;
        let store = AuthStore::open_at(scratch("validity").join("auth.json"))
            .unwrap()
            .with_endpoints(Endpoints {
                anthropic_token: server.url("/v1/oauth/token"),
                ..Endpoints::default()
            });
        let ten_minutes = now_ms() + 10 * 60 * 1000;
        store.set(ANTHROPIC, oauth("current", ten_minutes)).unwrap();

        assert_eq!(store.resolve(ANTHROPIC).await.unwrap().token(), "current");
        let thirty_minutes = 30 * 60 * 1000;
        assert_eq!(
            store
                .resolve_valid_for(ANTHROPIC, thirty_minutes)
                .await
                .unwrap()
                .token(),
            "fresh"
        );
        assert_eq!(server.requests().len(), 1);
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
    async fn a_provider_without_a_refresh_grant_cannot_be_refreshed() {
        let store = AuthStore::open_at(scratch("no-refresh").join("auth.json")).unwrap();
        store.set("a-proxy", oauth("token", 1)).unwrap();

        let error = store.refresh("a-proxy").await.unwrap_err();
        assert!(matches!(error, AuthError::NoRefresh { .. }), "{error}");

        store.set(OPENROUTER, Credential::api_key("sk-or")).unwrap();
        let error = store.refresh(OPENROUTER).await.unwrap_err();
        assert!(matches!(error, AuthError::NoRefresh { .. }), "{error}");
    }

    /// The only test that sets the federation variables, so no other test sees them.
    #[tokio::test]
    async fn with_no_key_anthropic_federates_the_identity_token_and_caches_it() {
        let directory = scratch("federation");
        let token_file = directory.join("identity");
        fs::write(&token_file, "identity.jwt.token").unwrap();
        let server = TestServer::start(vec![(
            200,
            r#"{"access_token":"federated-token","expires_in":3600}"#.into(),
        )])
        .await;
        let store = AuthStore::open_at(directory.join("auth.json"))
            .unwrap()
            .with_endpoints(Endpoints {
                anthropic_api: server.base.clone(),
                ..Endpoints::default()
            });

        let keys: Vec<(String, Option<String>)> = env_names(ANTHROPIC)
            .into_iter()
            .map(|name| {
                let value = std::env::var(&name).ok();
                (name, value)
            })
            .collect();
        for (name, _) in &keys {
            std::env::remove_var(name);
        }
        std::env::set_var(anthropic::FEDERATION_RULE_ID_ENV, "fdrl_test");
        std::env::set_var(anthropic::ORGANIZATION_ID_ENV, "org_test");
        std::env::set_var(anthropic::IDENTITY_TOKEN_FILE_ENV, &token_file);

        let status = store.status_of(ANTHROPIC);
        let first = store.resolve(ANTHROPIC).await;
        let second = store.resolve(ANTHROPIC).await;

        for name in [
            anthropic::FEDERATION_RULE_ID_ENV,
            anthropic::ORGANIZATION_ID_ENV,
            anthropic::IDENTITY_TOKEN_FILE_ENV,
        ] {
            std::env::remove_var(name);
        }
        for (name, value) in keys {
            if let Some(value) = value {
                std::env::set_var(name, value);
            }
        }

        assert_eq!(status.source, CredentialSource::Federation);
        assert_eq!(first.unwrap().token(), "federated-token");
        assert_eq!(second.unwrap().token(), "federated-token");
        assert_eq!(
            server.requests().len(),
            1,
            "the token is reused while fresh"
        );
        assert!(anthropic::is_federated_token("federated-token"));
        assert_eq!(
            server.requests()[0].json()["assertion"],
            "identity.jwt.token"
        );
    }
}
