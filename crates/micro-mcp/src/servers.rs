//! Every configured server, connected side by side, and where each one stands.

use crate::config;
use crate::config::Exposure;
use crate::config::LoadedConfig;
use crate::config::ServerEntry;
use crate::config::Transport as Configured;
use crate::names;
use crate::oauth;
use crate::oauth::Challenge;
use crate::oauth::CredentialStore;
use crate::transport::Authorizer;
use crate::transport::HttpTransport;
use crate::transport::StdioTransport;
use crate::transport::Transport;
use crate::transport::TransportError;
use crate::Client;
use crate::McpError;
use crate::Result;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

/// How long to wait before each further attempt to connect to an HTTP server that failed in a
/// way another attempt may not.
const CONNECT_RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(250), Duration::from_secs(1)];

/// Characters of one server's summary in the system prompt.
const MAX_SUMMARY_CHARS: usize = 250;

/// Characters of the whole system prompt section. Summaries shrink to fit; past that, the last
/// servers are counted instead of listed.
const MAX_SECTION_CHARS: usize = 4096;

/// Where one server stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Configured but turned off.
    Disabled,
    /// Not connected yet.
    Connecting,
    Connected {
        tools: usize,
    },
    /// The server wants a sign-in before it will talk.
    NeedsSignIn,
    /// Signed out during this session.
    SignedOut,
    Failed(String),
}

impl Status {
    /// The state in a few words.
    pub fn label(&self) -> String {
        match self {
            Status::Disabled => "disabled".to_string(),
            Status::Connecting => "connecting".to_string(),
            Status::Connected { tools: 1 } => "connected, 1 tool".to_string(),
            Status::Connected { tools } => format!("connected, {tools} tools"),
            Status::NeedsSignIn => "needs sign-in".to_string(),
            Status::SignedOut => "signed out".to_string(),
            Status::Failed(error) => {
                format!("failed: {}", error.lines().next().unwrap_or_default())
            }
        }
    }

    /// Whether the user has something to do about it.
    pub fn needs_attention(&self) -> bool {
        matches!(self, Status::NeedsSignIn | Status::Failed(_))
    }
}

/// One server, as `/mcp` and `micro mcp list` show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerReport {
    pub name: String,
    pub scope: config::Scope,
    pub source: PathBuf,
    /// What it runs or where it is.
    pub target: String,
    pub enabled: bool,
    pub exposure: Option<Exposure>,
    pub description: Option<String>,
    pub status: Status,
    /// The names the model knows its tools by.
    pub tools: Vec<String>,
    /// Whether `micro mcp login` applies to it.
    pub signs_in: bool,
    /// Whether credentials are stored for it.
    pub signed_in: bool,
}

struct State {
    entry: ServerEntry,
    status: Status,
    client: Option<Arc<Client>>,
    tools: Vec<String>,
    /// What the server last asked for when it refused a token.
    challenge: Arc<Mutex<Option<Challenge>>>,
}

/// The configured servers, shared by whatever connects, lists, or signs in to them.
#[derive(Clone)]
pub struct Servers {
    inner: Arc<Inner>,
}

struct Inner {
    workspace: PathBuf,
    credentials: Option<CredentialStore>,
    providers: Option<Arc<micro_auth::AuthStore>>,
    allowed: Vec<String>,
    excluded: Vec<String>,
    errors: Vec<String>,
    states: Mutex<BTreeMap<String, State>>,
    arrivals: micro_tools::Arrivals,
}

impl Servers {
    /// The servers `config` names, none of them connected yet. Relative working directories are
    /// taken from `workspace`.
    pub fn new(config: LoadedConfig, workspace: &Path) -> Servers {
        let states = config
            .servers
            .into_iter()
            .map(|entry| {
                let status = match entry.config.enabled {
                    true => Status::Connecting,
                    false => Status::Disabled,
                };
                (
                    entry.name.clone(),
                    State {
                        entry,
                        status,
                        client: None,
                        tools: Vec::new(),
                        challenge: Arc::default(),
                    },
                )
            })
            .collect();
        Servers {
            inner: Arc::new(Inner {
                workspace: workspace.to_path_buf(),
                credentials: None,
                providers: None,
                allowed: Vec::new(),
                excluded: Vec::new(),
                errors: config.errors,
                states: Mutex::new(states),
                arrivals: micro_tools::Arrivals::default(),
            }),
        }
    }

    fn configure(mut self, change: impl FnOnce(&mut Inner)) -> Servers {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            change(inner);
        }
        self
    }

    /// Keep OAuth sign-ins in `store`.
    pub fn with_credentials(self, store: CredentialStore) -> Servers {
        self.configure(|inner| inner.credentials = Some(store))
    }

    /// Read provider credentials for `"auth": { "provider": … }` servers from `store`.
    pub fn with_providers(self, store: Arc<micro_auth::AuthStore>) -> Servers {
        self.configure(|inner| inner.providers = Some(store))
    }

    /// Offer only tools named in `allowed` (when it names any), and none named in `excluded`.
    pub fn with_tool_filter(self, allowed: Vec<String>, excluded: Vec<String>) -> Servers {
        self.configure(|inner| {
            inner.allowed = allowed;
            inner.excluded = excluded;
        })
    }

    /// Where the tools of servers that connect in the background are delivered.
    pub fn arrivals(&self) -> micro_tools::Arrivals {
        self.inner.arrivals.clone()
    }

    /// What was wrong with the configuration files.
    pub fn errors(&self) -> &[String] {
        &self.inner.errors
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    pub fn entries(&self) -> Vec<ServerEntry> {
        self.lock()
            .values()
            .map(|state| state.entry.clone())
            .collect()
    }

    pub fn entry(&self, name: &str) -> Option<ServerEntry> {
        self.lock().get(name).map(|state| state.entry.clone())
    }

    pub fn status(&self, name: &str) -> Option<Status> {
        self.lock().get(name).map(|state| state.status.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, State>> {
        self.inner
            .states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, name: &str, change: impl FnOnce(&mut State)) {
        if let Some(state) = self.lock().get_mut(name) {
            change(state);
        }
    }

    /// Connect one server and collect its tools. A server turned off offers none.
    pub async fn connect(&self, name: &str) -> Result<Vec<Arc<dyn micro_tools::Tool>>> {
        let Some(entry) = self.entry(name) else {
            return Err(McpError::Config {
                server: name.to_string(),
                message: "is not configured".to_string(),
            });
        };
        if !entry.config.enabled {
            self.update(name, |state| state.status = Status::Disabled);
            return Ok(Vec::new());
        }
        self.update(name, |state| state.status = Status::Connecting);

        let retries: &[Duration] = match entry.config.url() {
            Some(_) => &CONNECT_RETRY_DELAYS,
            None => &[],
        };
        let mut attempt = 0;
        let tools = loop {
            let tools = self.open_with_tools(&entry).await;
            match (&tools, retries.get(attempt)) {
                (Err(error), Some(delay)) if error.is_transient() => {
                    attempt += 1;
                    tokio::time::sleep(*delay).await;
                }
                _ => break tools,
            }
        };

        match tools {
            Ok((client, tools)) => {
                let tools: Vec<Arc<dyn micro_tools::Tool>> = tools
                    .into_iter()
                    .filter(|tool| self.offers(&tool.definition().name))
                    .collect();
                let names = tools.iter().map(|tool| tool.definition().name).collect();
                self.update(name, |state| {
                    state.status = Status::Connected { tools: tools.len() };
                    state.client = Some(client);
                    state.tools = names;
                });
                Ok(tools)
            }
            Err(error) => {
                let status = match &error {
                    McpError::AuthRequired { .. } => Status::NeedsSignIn,
                    other => Status::Failed(other.to_string()),
                };
                self.update(name, |state| {
                    state.status = status;
                    state.client = None;
                    state.tools.clear();
                });
                Err(error)
            }
        }
    }

    /// Connect once, and list the server's tools.
    async fn open_with_tools(
        &self,
        entry: &ServerEntry,
    ) -> Result<(Arc<Client>, Vec<Arc<dyn micro_tools::Tool>>)> {
        let client = self.open(entry).await?;
        let tools = client
            .tools(
                |tool| entry.config.exposure_of(tool),
                entry.config.description.clone(),
            )
            .await?;
        Ok((client, tools))
    }

    fn offers(&self, tool: &str) -> bool {
        let listed =
            self.inner.allowed.is_empty() || self.inner.allowed.iter().any(|name| name == tool);
        listed && !self.inner.excluded.iter().any(|name| name == tool)
    }

    /// Connect every server that is turned on, side by side, and collect their tools.
    pub async fn connect_all(&self) -> (Vec<Arc<dyn micro_tools::Tool>>, Vec<McpError>) {
        let names: Vec<String> = self
            .entries()
            .into_iter()
            .filter(|entry| entry.config.enabled)
            .map(|entry| entry.name)
            .collect();
        self.connect_each(&names).await
    }

    /// Connect the named servers side by side, and collect their tools.
    pub async fn connect_each(
        &self,
        names: &[String],
    ) -> (Vec<Arc<dyn micro_tools::Tool>>, Vec<McpError>) {
        let outcomes = futures::future::join_all(names.iter().map(|name| self.connect(name))).await;
        let mut tools = Vec::new();
        let mut problems = Vec::new();
        for outcome in outcomes {
            match outcome {
                Ok(found) => tools.extend(found),
                Err(error) => problems.push(error),
            }
        }
        (tools, problems)
    }

    /// Connect a server without waiting for it. Its tools are delivered to the arrivals, and a
    /// tool search waits for them.
    pub fn connect_in_background(&self, name: &str) {
        let expected = self.inner.arrivals.expect(names::namespace(name));
        let servers = self.clone();
        let name = name.to_string();
        tokio::spawn(async move {
            if let Ok(tools) = servers.connect(&name).await {
                servers.inner.arrivals.add(tools);
            }
            drop(expected);
        });
    }

    /// Connect a server again, for example after signing in, delivering its tools to the
    /// arrivals. Says how many tools it offers.
    pub async fn reconnect(&self, name: &str) -> Result<usize> {
        let expected = self.inner.arrivals.expect(names::namespace(name));
        self.inner
            .arrivals
            .remove_prefixed(&format!("{}__", names::namespace(name)));
        let connected = self.connect(name).await;
        drop(expected);
        let tools = connected?;
        let count = tools.len();
        self.inner.arrivals.add(tools);
        Ok(count)
    }

    async fn open(&self, entry: &ServerEntry) -> Result<Arc<Client>> {
        let name = entry.name.as_str();
        let config_error = |message: String| McpError::Config {
            server: name.to_string(),
            message,
        };
        let (sender, incoming) = tokio::sync::mpsc::unbounded_channel();

        let transport: Arc<dyn Transport> = match &entry.config.transport {
            Configured::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let env = env
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), config::resolve_value(value)?)))
                    .collect::<std::result::Result<BTreeMap<_, _>, String>>()
                    .map_err(config_error)?;
                let command = expand_home(command);
                let args: Vec<String> = args.iter().map(|arg| expand_home(arg)).collect();
                let cwd = cwd
                    .as_deref()
                    .map(|cwd| self.inner.workspace.join(expand_home(cwd)));
                let spawned = StdioTransport::spawn(&command, &args, &env, cwd.as_deref(), sender)
                    .map_err(|error| McpError::Start {
                        server: name.to_string(),
                        command: command.clone(),
                        message: error.to_string(),
                    })?;
                Arc::new(spawned)
            }
            Configured::Http {
                url,
                headers,
                oauth,
                provider,
            } => {
                let parsed = reqwest::Url::parse(url)
                    .map_err(|_| config_error(format!("invalid url {url}")))?;
                let headers = headers
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), config::resolve_value(value)?)))
                    .collect::<std::result::Result<Vec<_>, String>>()
                    .map_err(config_error)?;
                let authorizer: Option<Arc<dyn Authorizer>> = match provider {
                    Some(provider) => {
                        let store = match &self.inner.providers {
                            Some(store) => Arc::clone(store),
                            None => Arc::new(
                                micro_auth::AuthStore::open()
                                    .map_err(|error| config_error(error.to_string()))?,
                            ),
                        };
                        Some(Arc::new(ProviderAuthorizer::new(provider, store)))
                    }
                    None if entry.config.uses_oauth() => match &self.inner.credentials {
                        Some(store) => {
                            let authorizer = oauth::OAuthAuthorizer::new(
                                parsed.clone(),
                                oauth.clone(),
                                store.server(name, url),
                            );
                            let challenges = authorizer.challenges();
                            self.update(name, |state| state.challenge = challenges);
                            Some(Arc::new(authorizer))
                        }
                        None => None,
                    },
                    None => None,
                };
                Arc::new(HttpTransport::new(parsed, headers, authorizer, sender))
            }
        };

        Client::connect(name, transport, incoming, entry.config.timeout).await
    }

    /// Every server and where it stands, servers that need attention first.
    pub fn report(&self) -> Vec<ServerReport> {
        let mut reports: Vec<ServerReport> = self
            .lock()
            .values()
            .map(|state| {
                let config = &state.entry.config;
                let signed_in = match (&self.inner.credentials, config.url()) {
                    (Some(store), Some(url)) if config.uses_oauth() => store
                        .server(&state.entry.name, url)
                        .load()
                        .is_some_and(|stored| stored.tokens.is_some()),
                    _ => false,
                };
                ServerReport {
                    name: state.entry.name.clone(),
                    scope: state.entry.scope,
                    source: state.entry.source.clone(),
                    target: config.target(),
                    enabled: config.enabled,
                    exposure: config.exposure,
                    description: config.description.clone(),
                    status: state.status.clone(),
                    tools: state.tools.clone(),
                    signs_in: config.uses_oauth(),
                    signed_in,
                }
            })
            .collect();
        reports.sort_by_key(|report| !report.status.needs_attention());
        reports
    }

    /// Start signing in to a server, answering whatever it last asked for.
    pub async fn begin_sign_in(&self, name: &str) -> std::result::Result<oauth::SignIn, String> {
        let entry = self
            .entry(name)
            .ok_or_else(|| format!("no MCP server is called {name}"))?;
        let (Some(url), true) = (entry.config.url(), entry.config.uses_oauth()) else {
            return Err(format!(
                "{name} does not sign in with OAuth: it is not an HTTP server, or it brings its \
                 own credential"
            ));
        };
        let store = self
            .inner
            .credentials
            .clone()
            .ok_or("there is nowhere to keep credentials; set MICRO_DIR")?;
        let challenge = self.lock().get(name).and_then(|state| {
            state
                .challenge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        });
        let config::Transport::Http { oauth, .. } = &entry.config.transport else {
            unreachable!("a server with a URL is an HTTP server");
        };
        oauth::begin_sign_in(url, oauth, store.server(name, url), challenge.as_ref()).await
    }

    /// Forget a server's stored sign-in. Says whether there was one.
    pub fn sign_out(&self, name: &str) -> std::result::Result<bool, String> {
        let entry = self
            .entry(name)
            .ok_or_else(|| format!("no MCP server is called {name}"))?;
        let url = entry
            .config
            .url()
            .ok_or_else(|| format!("{name} is not an HTTP server"))?;
        let store = self
            .inner
            .credentials
            .clone()
            .ok_or("there is nowhere credentials are kept")?;
        let removed = store
            .server(name, url)
            .remove()
            .map_err(|error| error.to_string())?;
        self.inner
            .arrivals
            .remove_prefixed(&format!("{}__", names::namespace(name)));
        self.update(name, |state| {
            state.status = Status::SignedOut;
            state.client = None;
            state.tools.clear();
        });
        Ok(removed)
    }

    /// The `mcp_servers` system prompt section: the named servers, whose tools are not declared
    /// to the model, each with a line on what it offers, and how their tools are reached: from
    /// `codemode` scripts when `codemode` is offered, and through `tool_search`. `None` when there
    /// are none.
    pub fn prompt_section(&self, undeclared: &[String], codemode: bool) -> Option<String> {
        let states = self.lock();
        let mut listed: Vec<(String, String)> = undeclared
            .iter()
            .filter_map(|name| states.get(name))
            .filter(|state| state.entry.config.enabled)
            .map(|state| (names::namespace(&state.entry.name), summary(state)))
            .collect();
        if listed.is_empty() {
            return None;
        }
        listed.sort();

        let intro = match codemode {
            true => {
                "MCP servers whose tools are not declared to you. Call their tools from `codemode` \
                 scripts, finding them with `searchTools()` or `describeNamespace(\"<server>\")`, \
                 or load them with `tool_search`."
            }
            false => {
                "MCP servers whose tools are not declared to you. Load their tools with \
                 `tool_search`."
            }
        };
        let omitted = |count: usize| match count {
            0 => Vec::new(),
            1 => vec!["- … 1 more server".to_string()],
            count => vec![format!("- … {count} more servers")],
        };
        let size = |kept: usize| {
            std::iter::once(intro.to_string())
                .chain(listed[..kept].iter().map(|(head, _)| format!("- {head}")))
                .chain(omitted(listed.len() - kept))
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .count()
        };
        let mut kept = listed.len();
        while kept > 0 && size(kept) > MAX_SECTION_CHARS {
            kept -= 1;
        }
        let per_server = match kept {
            0 => 0,
            kept => MAX_SUMMARY_CHARS
                .min((MAX_SECTION_CHARS - size(kept)) / kept)
                .saturating_sub(2),
        };

        let mut lines = vec![intro.to_string()];
        for (head, summary) in &listed[..kept] {
            let summary = truncate(summary, per_server);
            lines.push(match summary.is_empty() {
                true => format!("- {head}"),
                false => format!("- {head}: {summary}"),
            });
        }
        lines.extend(omitted(listed.len() - kept));
        Some(lines.join("\n"))
    }
}

/// The first line of the configured description, or of what the server said about itself.
fn summary(state: &State) -> String {
    let text = state
        .entry
        .config
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .or_else(|| {
            state
                .client
                .as_ref()
                .and_then(|client| client.instructions())
        })
        .unwrap_or_default();
    text.lines().next().unwrap_or_default().trim().to_string()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max <= 1 {
        return String::new();
    }
    let kept: String = text.chars().take(max - 1).collect();
    format!("{}…", kept.trim_end())
}

/// A leading `~/` names the home directory.
fn expand_home(value: &str) -> String {
    match (value.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest).display().to_string(),
        _ => value.to_string(),
    }
}

/// Sends a provider's current micro credential as the bearer token, read on every request so a
/// refreshed credential applies at once.
pub struct ProviderAuthorizer {
    provider: String,
    store: Arc<micro_auth::AuthStore>,
}

impl ProviderAuthorizer {
    pub fn new(provider: &str, store: Arc<micro_auth::AuthStore>) -> ProviderAuthorizer {
        ProviderAuthorizer {
            provider: provider.to_string(),
            store,
        }
    }
}

#[async_trait]
impl Authorizer for ProviderAuthorizer {
    async fn token(&self) -> std::result::Result<Option<String>, TransportError> {
        self.store
            .resolve(&self.provider)
            .await
            .map(|credential| Some(credential.token().to_string()))
            .map_err(|error| {
                TransportError::Other(format!(
                    "no {} credential to send ({error}); run `micro auth login {}`",
                    self.provider, self.provider
                ))
            })
    }

    async fn unauthorized(
        &self,
        _challenge: &Challenge,
        _rejected: Option<&str>,
    ) -> std::result::Result<(), TransportError> {
        self.store
            .refresh(&self.provider)
            .await
            .map(|_| ())
            .map_err(|_| {
                TransportError::Other(format!(
                    "the server refused the {} credential",
                    self.provider
                ))
            })
    }
}
