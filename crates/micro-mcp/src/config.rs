//! Where servers are configured: `mcp.json` in micro's configuration directory and, in a trusted
//! project, `.micro/mcp.json`. Both use the `mcpServers` shape other MCP clients share, so an
//! entry can be copied from one to the other. A project entry replaces a global one of the same
//! name.

use crate::names;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

/// What the configuration file is called, in either place.
pub const FILE_NAME: &str = "mcp.json";

/// The key the servers sit under.
const SERVERS_KEY: &str = "mcpServers";

/// Hosts a credential may be sent to over plain HTTP, since the traffic never leaves the machine.
const LOOPBACK_HOSTS: &[&str] = &["localhost", "127.0.0.1", "[::1]", "::1"];

/// How a server's tools reach the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// Declared to the model like any built-in tool. The first prompt waits for the server.
    Direct,
    /// Left out until `tool_search` finds them. The server connects in the background.
    Deferred,
}

impl Exposure {
    pub fn name(self) -> &'static str {
        match self {
            Exposure::Direct => "direct",
            Exposure::Deferred => "deferred",
        }
    }

    fn parse(value: &str) -> Option<Exposure> {
        match value {
            "direct" => Some(Exposure::Direct),
            "deferred" => Some(Exposure::Deferred),
            _ => None,
        }
    }
}

/// OAuth client settings, for servers that need more than dynamic client registration with
/// defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OAuthConfig {
    /// A client registered ahead of time. Without one, micro registers itself.
    pub client_id: Option<String>,
    /// May name an environment variable (`${NAME}`) or a command (`!cmd`).
    pub client_secret: Option<String>,
    /// The loopback port the browser is sent back to, for a client registered with a fixed one.
    pub callback_port: Option<u16>,
    /// The redirect URI registered for `client_id`: `http` on a loopback host.
    pub callback_url: Option<String>,
    /// Scopes to ask for, separated by spaces, beyond those the server advertises.
    pub scope: Option<String>,
    /// The `client_name` sent when micro registers itself, for servers that only accept clients
    /// they know.
    pub client_name: Option<String>,
    /// The authorization server's metadata document, used instead of discovery for servers that
    /// advertise the wrong authorization server or none. Trusted as configured.
    pub auth_server_metadata_url: Option<String>,
}

/// How a server is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// A program micro starts, spoken to over its standard input and output.
    Stdio {
        command: String,
        args: Vec<String>,
        /// Values may name environment variables (`${NAME}`) or commands (`!cmd`).
        env: BTreeMap<String, String>,
        /// Relative to the workspace.
        cwd: Option<String>,
    },
    /// A streamable HTTP endpoint.
    Http {
        url: String,
        /// Values may name environment variables (`${NAME}`) or commands (`!cmd`).
        headers: BTreeMap<String, String>,
        oauth: OAuthConfig,
        /// Send this provider's micro credential as the bearer token instead of using OAuth.
        provider: Option<String>,
    },
}

/// One server, as a configuration file describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub transport: Transport,
    /// What the server offers, in a sentence, for the system prompt.
    pub description: Option<String>,
    /// A server turned off stays in the file, so it can be turned back on.
    pub enabled: bool,
    /// How long any one request may take.
    pub timeout: Option<Duration>,
    /// Unset lets the tool search threshold decide.
    pub exposure: Option<Exposure>,
}

impl ServerConfig {
    /// A stdio server running `command`.
    pub fn stdio(command: impl Into<String>, args: Vec<String>) -> Self {
        ServerConfig {
            transport: Transport::Stdio {
                command: command.into(),
                args,
                env: BTreeMap::new(),
                cwd: None,
            },
            description: None,
            enabled: true,
            timeout: None,
            exposure: None,
        }
    }

    /// An HTTP server at `url`.
    pub fn http(url: impl Into<String>) -> Self {
        ServerConfig {
            transport: Transport::Http {
                url: url.into(),
                headers: BTreeMap::new(),
                oauth: OAuthConfig::default(),
                provider: None,
            },
            description: None,
            enabled: true,
            timeout: None,
            exposure: None,
        }
    }

    /// The URL of an HTTP server.
    pub fn url(&self) -> Option<&str> {
        match &self.transport {
            Transport::Http { url, .. } => Some(url),
            Transport::Stdio { .. } => None,
        }
    }

    /// Whether micro may sign in to the server with OAuth: an HTTP server that brings no
    /// credential of its own.
    pub fn uses_oauth(&self) -> bool {
        match &self.transport {
            Transport::Http {
                headers, provider, ..
            } => {
                provider.is_none()
                    && !headers
                        .keys()
                        .any(|name| name.eq_ignore_ascii_case("authorization"))
            }
            Transport::Stdio { .. } => false,
        }
    }

    /// What it runs or where it is, in a line.
    pub fn target(&self) -> String {
        match &self.transport {
            Transport::Stdio { command, args, .. } => std::iter::once(command.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
            Transport::Http { url, .. } => url.clone(),
        }
    }
}

/// Which file an entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Project,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::Project => "project",
        }
    }
}

/// A configured server and where it was configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEntry {
    pub name: String,
    pub config: ServerConfig,
    pub source: PathBuf,
    pub scope: Scope,
}

/// Every configured server, and what was wrong with the entries that were left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadedConfig {
    pub servers: Vec<ServerEntry>,
    pub errors: Vec<String>,
}

impl LoadedConfig {
    pub fn get(&self, name: &str) -> Option<&ServerEntry> {
        self.servers.iter().find(|entry| entry.name == name)
    }
}

/// The global `mcp.json`, in micro's configuration directory.
pub fn global_path() -> Option<PathBuf> {
    micro_dirs::config_dir().map(|directory| directory.join(FILE_NAME))
}

/// A project's `mcp.json`.
pub fn project_path(workspace: &Path) -> PathBuf {
    workspace.join(".micro").join(FILE_NAME)
}

/// Read the global file and, when the project is trusted, the project's.
pub fn load(workspace: &Path, project_trusted: bool) -> LoadedConfig {
    let project = project_path(workspace);
    load_from(
        global_path().as_deref(),
        project_trusted.then_some(project.as_path()),
    )
}

/// Read the given files, the project's last so its entries replace the global ones.
pub fn load_from(global: Option<&Path>, project: Option<&Path>) -> LoadedConfig {
    let mut loaded = LoadedConfig::default();
    if let Some(path) = global {
        read_file(path, Scope::Global, &mut loaded);
    }
    if let Some(path) = project {
        read_file(path, Scope::Project, &mut loaded);
    }
    loaded
}

fn read_file(path: &Path, scope: Scope, loaded: &mut LoadedConfig) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            loaded.errors.push(format!("{}: {error}", path.display()));
            return;
        }
    };
    let servers = match parse_file(&text) {
        Ok(servers) => servers,
        Err(error) => {
            loaded.errors.push(format!("{}: {error}", path.display()));
            return;
        }
    };

    for (name, value) in servers {
        let config = match parse_server(&name, &value) {
            Ok(config) => config,
            Err(error) => {
                loaded.errors.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        if scope == Scope::Project
            && matches!(
                config.transport,
                Transport::Http {
                    provider: Some(_),
                    ..
                }
            )
        {
            loaded.errors.push(format!(
                "{}: server \"{name}\": auth is only allowed in the global {FILE_NAME}",
                path.display()
            ));
            continue;
        }
        let clash = loaded.servers.iter().find(|other| {
            other.name != name && names::namespace(&other.name) == names::namespace(&name)
        });
        if let Some(clash) = clash {
            loaded.errors.push(format!(
                "{}: server \"{name}\" conflicts with \"{}\"",
                path.display(),
                clash.name
            ));
            continue;
        }
        let entry = ServerEntry {
            name: name.clone(),
            config,
            source: path.to_path_buf(),
            scope,
        };
        match loaded.servers.iter_mut().find(|other| other.name == name) {
            Some(replaced) => *replaced = entry,
            None => loaded.servers.push(entry),
        }
    }
}

/// The server entries of a file's text, in the order written.
fn parse_file(text: &str) -> Result<Vec<(String, Value)>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let shape = || format!("expected an object with an \"{SERVERS_KEY}\" object");
    let object = parsed.as_object().ok_or_else(shape)?;
    match object.get(SERVERS_KEY) {
        None => Ok(Vec::new()),
        Some(Value::Object(servers)) => Ok(servers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()),
        Some(_) => Err(shape()),
    }
}

/// Check one entry of the `mcpServers` shape and read it.
pub fn parse_server(name: &str, value: &Value) -> Result<ServerConfig, String> {
    if !names::is_valid_server_name(name) {
        return Err(format!(
            "invalid server name \"{name}\" (use letters, digits, \"_\" and \"-\")"
        ));
    }
    let object = value
        .as_object()
        .ok_or_else(|| format!("server \"{name}\" must be an object"))?;
    let problem = |message: &str| format!("server \"{name}\": {message}");

    let enabled = match object.get("enabled") {
        None => true,
        Some(Value::Bool(enabled)) => *enabled,
        Some(_) => return Err(problem("enabled must be a boolean")),
    };
    let description = optional_string(object, "description").map_err(|error| problem(&error))?;
    let timeout = match object.get("timeout") {
        None => None,
        Some(value) => match value.as_f64() {
            Some(seconds) if seconds > 0.0 => Some(Duration::from_secs_f64(seconds)),
            _ => return Err(problem("timeout must be a positive number of seconds")),
        },
    };
    let exposure = match object.get("exposure") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .and_then(Exposure::parse)
                .ok_or_else(|| problem("exposure must be \"direct\" or \"deferred\""))?,
        ),
    };

    let kind = optional_string(object, "type").map_err(|error| problem(&error))?;
    if kind.as_deref() == Some("sse") {
        return Err(problem(
            "the SSE transport is not supported; use the server's streamable HTTP URL",
        ));
    }

    let transport = match (object.get("url"), object.get("command"), kind.as_deref()) {
        (Some(Value::String(url)), _, None | Some("http") | Some("streamable-http")) => {
            http_transport(object, url).map_err(|error| problem(&error))?
        }
        (_, Some(Value::String(command)), None | Some("stdio")) => {
            stdio_transport(object, command).map_err(|error| problem(&error))?
        }
        (_, _, Some(other)) if !["http", "streamable-http", "stdio"].contains(&other) => {
            return Err(problem(
                "type must be \"stdio\", \"http\", or \"streamable-http\"",
            ))
        }
        _ => {
            return Err(format!(
                "server \"{name}\" needs either \"command\" (stdio) or \"url\" (streamable HTTP)"
            ))
        }
    };

    Ok(ServerConfig {
        transport,
        description,
        enabled,
        timeout,
        exposure,
    })
}

fn stdio_transport(object: &Map<String, Value>, command: &str) -> Result<Transport, String> {
    if command.trim().is_empty() {
        return Err("command must not be empty".to_string());
    }
    let args = match object.get("args") {
        None => Vec::new(),
        Some(Value::Array(args)) => args
            .iter()
            .map(|arg| arg.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or("args must be an array of strings")?,
        Some(_) => return Err("args must be an array of strings".to_string()),
    };
    Ok(Transport::Stdio {
        command: command.to_string(),
        args,
        env: string_map(object, "env")?,
        cwd: optional_string(object, "cwd")?,
    })
}

fn http_transport(object: &Map<String, Value>, url: &str) -> Result<Transport, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "url must be an http or https URL")?;
    if !["http", "https"].contains(&parsed.scheme()) {
        return Err("url must be an http or https URL".to_string());
    }

    let provider = match object.get("auth") {
        None => None,
        Some(auth) => {
            let provider = auth
                .get("provider")
                .and_then(Value::as_str)
                .filter(|provider| !provider.trim().is_empty())
                .ok_or("auth.provider must be a provider name")?;
            if !secure_or_loopback(&parsed) {
                return Err(
                    "auth requires an https URL, or http on localhost, 127.0.0.1, or [::1]"
                        .to_string(),
                );
            }
            Some(provider.to_string())
        }
    };

    Ok(Transport::Http {
        url: url.to_string(),
        headers: string_map(object, "headers")?,
        oauth: oauth_config(object.get("oauth"))?,
        provider,
    })
}

fn oauth_config(value: Option<&Value>) -> Result<OAuthConfig, String> {
    let Some(value) = value else {
        return Ok(OAuthConfig::default());
    };
    let object = value.as_object().ok_or("oauth must be an object")?;
    let field = |key: &str| {
        optional_string(object, key).map_err(|_| format!("oauth.{key} must be a string"))
    };

    let callback_port = match object.get("callbackPort") {
        None => None,
        Some(port) => Some(
            port.as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port > 0)
                .ok_or("oauth.callbackPort must be a port number")?,
        ),
    };

    let callback_url = field("callbackUrl")?;
    if let Some(callback_url) = &callback_url {
        let parsed = reqwest::Url::parse(callback_url)
            .ok()
            .filter(is_loopback_redirect)
            .ok_or(
                "oauth.callbackUrl must be an http URI on localhost, 127.0.0.1, or [::1] \
                 without query or fragment",
            )?;
        if let (Some(in_url), Some(port)) = (parsed.port(), callback_port) {
            if in_url != port {
                return Err("oauth.callbackUrl and oauth.callbackPort name different ports".into());
            }
        }
    }

    let client_name = field("clientName")?;
    if client_name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err("oauth.clientName must be a non-empty string".to_string());
    }

    let auth_server_metadata_url = field("authServerMetadataUrl")?;
    if let Some(url) = &auth_server_metadata_url {
        if !reqwest::Url::parse(url).is_ok_and(|parsed| secure_or_loopback(&parsed)) {
            return Err(
                "oauth.authServerMetadataUrl must be an https URL, or http on \
                        localhost, 127.0.0.1, or [::1]"
                    .to_string(),
            );
        }
    }

    Ok(OAuthConfig {
        client_id: field("clientId")?,
        client_secret: field("clientSecret")?,
        callback_port,
        callback_url,
        scope: field("scope")?,
        client_name,
        auth_server_metadata_url,
    })
}

/// Whether a URL is https, or http to this machine.
pub fn secure_or_loopback(url: &reqwest::Url) -> bool {
    url.scheme() == "https" || (url.scheme() == "http" && is_loopback(url))
}

pub fn is_loopback(url: &reqwest::Url) -> bool {
    url.host_str()
        .is_some_and(|host| LOOPBACK_HOSTS.contains(&host))
}

/// Whether the loopback callback server can stand behind a redirect URI.
fn is_loopback_redirect(url: &reqwest::Url) -> bool {
    url.scheme() == "http" && is_loopback(url) && url.query().is_none() && url.fragment().is_none()
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn string_map(object: &Map<String, Value>, key: &str) -> Result<BTreeMap<String, String>, String> {
    match object.get(key) {
        None => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(name, value)| Some((name.clone(), value.as_str()?.to_string())))
            .collect::<Option<BTreeMap<_, _>>>()
            .ok_or_else(|| format!("{key} must map names to strings")),
        Some(_) => Err(format!("{key} must map names to strings")),
    }
}

/// The value a configured string stands for: the output of `!command` when it is one, otherwise
/// the string with every `${NAME}` replaced by that environment variable.
pub fn resolve_value(raw: &str) -> Result<String, String> {
    if let Some(command) = raw.strip_prefix('!') {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .stderr(std::process::Stdio::null())
            .output()
            .map_err(|error| format!("cannot run `{command}`: {error}"))?;
        if !output.status.success() {
            return Err(format!("`{command}` failed with {}", output.status));
        }
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    let mut resolved = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        resolved.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            resolved.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let name = &after[..end];
        let value = std::env::var(name)
            .map_err(|_| format!("the environment variable {name} is not set"))?;
        resolved.push_str(&value);
        rest = &after[end + 1..];
    }
    resolved.push_str(rest);
    Ok(resolved)
}

/// Add a server to a file, creating the file when it is missing and replacing an entry of the
/// same name. Says whether one was replaced. Everything else in the file is kept.
pub fn add_server(path: &Path, name: &str, server: Value) -> Result<bool, String> {
    let mut replaced = false;
    edit_servers(path, |servers| {
        replaced = servers.insert(name.to_string(), server).is_some();
        true
    })?;
    Ok(replaced)
}

/// Remove a server from a file. Says whether the file had it.
pub fn remove_server(path: &Path, name: &str) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let mut removed = false;
    edit_servers(path, |servers| {
        removed = servers.remove(name).is_some();
        removed
    })?;
    Ok(removed)
}

/// Read a file's `mcpServers`, let `edit` change them, and write the file back when it says so.
fn edit_servers(
    path: &Path,
    edit: impl FnOnce(&mut Map<String, Value>) -> bool,
) -> Result<(), String> {
    let shown = path.display();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("{shown}: {error}")),
    };
    let mut parsed = match text.trim().is_empty() {
        true => Value::Object(Map::new()),
        false => serde_json::from_str(&text).map_err(|error| format!("{shown}: {error}"))?,
    };
    let shape = || format!("{shown}: expected an object with an \"{SERVERS_KEY}\" object");
    let object = parsed.as_object_mut().ok_or_else(shape)?;
    let servers = object
        .entry(SERVERS_KEY)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(shape)?;
    if !edit(servers) {
        return Ok(());
    }

    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(|error| format!("{shown}: {error}"))?;
    }
    let written = serde_json::to_string_pretty(&parsed).map_err(|error| error.to_string())?;
    std::fs::write(path, format!("{written}\n")).map_err(|error| format!("{shown}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("micro-mcp-config-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn write(path: &Path, value: Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }

    #[test]
    fn a_command_is_stdio_and_a_url_is_http() {
        let stdio = parse_server(
            "fs",
            &json!({ "command": "npx", "args": ["-y", "server"], "env": { "A": "b" } }),
        )
        .unwrap();
        assert_eq!(
            stdio.transport,
            Transport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "server".into()],
                env: BTreeMap::from([("A".to_string(), "b".to_string())]),
                cwd: None,
            }
        );

        let http = parse_server(
            "docs",
            &json!({
                "url": "https://example.com/mcp",
                "headers": { "Authorization": "Bearer ${TOKEN}" },
                "description": "Search the docs",
                "oauth": { "clientName": "Claude Code" },
            }),
        )
        .unwrap();
        assert_eq!(http.url(), Some("https://example.com/mcp"));
        assert_eq!(http.description.as_deref(), Some("Search the docs"));
        assert!(!http.uses_oauth(), "it brings its own Authorization header");
    }

    #[test]
    fn entries_that_cannot_work_are_explained() {
        let cases = [
            (json!({ "type": "sse", "url": "https://x/sse" }), "SSE"),
            (json!({ "url": "ftp://x" }), "http or https"),
            (json!({ "args": [] }), "needs either"),
            (
                json!({ "command": "x", "exposure": "sometimes" }),
                "exposure",
            ),
            (
                json!({ "url": "http://example.com/mcp", "auth": { "provider": "openai" } }),
                "https",
            ),
            (
                json!({ "url": "https://x", "oauth": { "callbackUrl": "https://example.com/cb" } }),
                "callbackUrl",
            ),
            (
                json!({ "url": "https://x", "oauth": { "authServerMetadataUrl": "http://example.com/m" } }),
                "authServerMetadataUrl",
            ),
        ];
        for (value, expected) in cases {
            let error = parse_server("s", &value).expect_err(&value.to_string());
            assert!(
                error.contains(expected),
                "{error} should mention {expected}"
            );
        }
        assert!(parse_server("has space", &json!({ "command": "x" })).is_err());
    }

    #[test]
    fn a_provider_token_may_go_to_a_loopback_server_over_http() {
        let config = parse_server(
            "local",
            &json!({ "url": "http://127.0.0.1:9000/mcp", "auth": { "provider": "openai" } }),
        )
        .unwrap();
        assert!(!config.uses_oauth());
    }

    #[test]
    fn a_project_replaces_a_global_entry_but_cannot_pick_a_credential() {
        let directory = scratch("layers");
        let global = directory.join("global").join(FILE_NAME);
        let project = directory.join("project").join(FILE_NAME);
        write(
            &global,
            json!({ "mcpServers": {
                "shared": { "command": "global-server" },
                "mine": { "command": "mine" },
            }}),
        );
        write(
            &project,
            json!({ "mcpServers": {
                "shared": { "command": "project-server" },
                "creds": { "url": "https://example.com/mcp", "auth": { "provider": "openai" } },
                "my-server": { "command": "a" },
                "my_server": { "command": "b" },
            }}),
        );

        let loaded = load_from(Some(&global), Some(&project));
        let shared = loaded.get("shared").unwrap();
        assert_eq!(shared.scope, Scope::Project);
        assert_eq!(shared.config.target(), "project-server");
        assert!(loaded.get("mine").is_some());
        assert!(loaded.get("creds").is_none());
        assert!(
            loaded
                .errors
                .iter()
                .any(|error| error.contains("auth is only allowed")),
            "{:?}",
            loaded.errors
        );
        assert!(
            loaded
                .errors
                .iter()
                .any(|error| error.contains("conflicts with")),
            "{:?}",
            loaded.errors
        );

        let untrusted = load_from(Some(&global), None);
        assert_eq!(
            untrusted.get("shared").unwrap().config.target(),
            "global-server"
        );
    }

    #[test]
    fn adding_and_removing_keeps_the_rest_of_the_file() {
        let path = scratch("edit").join(FILE_NAME);
        write(
            &path,
            json!({ "other": true, "mcpServers": { "a": { "command": "a" } } }),
        );

        assert!(!add_server(&path, "b", json!({ "url": "https://b/mcp" })).unwrap());
        assert!(add_server(&path, "b", json!({ "url": "https://b2/mcp" })).unwrap());
        assert!(remove_server(&path, "a").unwrap());
        assert!(!remove_server(&path, "a").unwrap());

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["other"], true);
        assert_eq!(written["mcpServers"]["b"]["url"], "https://b2/mcp");
        assert!(written["mcpServers"].get("a").is_none());
    }

    #[test]
    fn values_name_variables_and_commands() {
        std::env::set_var("MICRO_MCP_TEST_TOKEN", "secret");
        assert_eq!(
            resolve_value("Bearer ${MICRO_MCP_TEST_TOKEN}").unwrap(),
            "Bearer secret"
        );
        assert_eq!(resolve_value("!echo hello").unwrap(), "hello");
        assert!(resolve_value("${MICRO_MCP_TEST_NOT_SET_ANYWHERE}").is_err());
        assert_eq!(resolve_value("plain").unwrap(), "plain");
    }
}
