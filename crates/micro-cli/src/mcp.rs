//! MCP servers from the shell (`micro mcp add|remove|list|login|logout`) and from a session
//! (`/mcp`). Shell commands need no session, so an agent can configure a server, check it, and
//! start a sign-in through bash; a running session picks up the new credentials when it next
//! reconnects the server.

use anyhow::bail;
use anyhow::Context as _;
use anyhow::Result;
use micro_commands::CommandOutcome;
use micro_commands::Picker;
use micro_commands::PickerItem;
use micro_mcp::config;
use micro_mcp::oauth::SignIn;
use micro_mcp::Servers;
use micro_mcp::Status;
use micro_tui::Applied;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::io::IsTerminal as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// How long a sign-in waits for the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// What `micro mcp add` was told about the server.
#[derive(Debug, Default)]
pub struct AddOptions {
    pub url: Option<String>,
    pub headers: Vec<String>,
    pub bearer_token_env_var: Option<String>,
    pub env: Vec<String>,
    pub cwd: Option<String>,
    pub description: Option<String>,
    pub exposure: Option<String>,
    pub oauth_client_id: Option<String>,
    pub oauth_client_secret: Option<String>,
    pub oauth_callback_port: Option<u16>,
    pub oauth_client_name: Option<String>,
    pub auth_provider: Option<String>,
    /// The program and its arguments, for a stdio server.
    pub command: Vec<String>,
}

/// Whether this workspace's own `mcp.json` may be read.
async fn trusted(root: &Path) -> bool {
    !micro_config::requires_decision(root)
        || micro_config::TrustStore::load()
            .await
            .unwrap_or_default()
            .is_trusted(root)
}

/// Every configured server, ready to connect or sign in to.
fn servers(root: &Path, loaded: config::LoadedConfig) -> Result<Servers> {
    let providers = Arc::new(micro_auth::AuthStore::open()?);
    let servers = Servers::new(loaded, root).with_providers(providers);
    let servers = match micro_mcp::ServerLog::in_data_dir() {
        Some(log) => servers.with_log(log),
        None => servers,
    };
    Ok(match micro_mcp::oauth::CredentialStore::open() {
        Some(credentials) => servers.with_credentials(credentials),
        None => servers,
    })
}

fn file_for(root: &Path, local: bool) -> Result<std::path::PathBuf> {
    match local {
        true => Ok(config::project_path(root)),
        false => config::global_path().context("no configuration directory; set MICRO_DIR or HOME"),
    }
}

/// `NAME=VALUE` pairs as a JSON object.
fn pairs(given: &[String], what: &str) -> Result<Map<String, Value>> {
    given
        .iter()
        .map(|pair| match pair.split_once('=') {
            Some((name, value)) if !name.trim().is_empty() => {
                Ok((name.trim().to_string(), json!(value)))
            }
            _ => bail!("{what} `{pair}` is not NAME=VALUE"),
        })
        .collect()
}

/// The `mcpServers` entry `micro mcp add` writes.
fn entry_for(options: &AddOptions) -> Result<Value> {
    let mut entry = Map::new();
    match (&options.url, options.command.split_first()) {
        (Some(_), Some(_)) => bail!("give either --url or a command after --, not both"),
        (None, None) => {
            bail!("give a command after -- for a stdio server, or --url for an HTTP one")
        }
        (Some(url), None) => {
            entry.insert("url".into(), json!(url));
            let mut headers = pairs(&options.headers, "header")?;
            if let Some(variable) = &options.bearer_token_env_var {
                headers.insert(
                    "Authorization".into(),
                    json!(format!("Bearer ${{{variable}}}")),
                );
            }
            if !headers.is_empty() {
                entry.insert("headers".into(), Value::Object(headers));
            }
            let mut oauth = Map::new();
            for (key, value) in [
                (
                    "clientId",
                    options.oauth_client_id.as_ref().map(|v| json!(v)),
                ),
                (
                    "clientSecret",
                    options.oauth_client_secret.as_ref().map(|v| json!(v)),
                ),
                (
                    "callbackPort",
                    options.oauth_callback_port.map(|v| json!(v)),
                ),
                (
                    "clientName",
                    options.oauth_client_name.as_ref().map(|v| json!(v)),
                ),
            ] {
                if let Some(value) = value {
                    oauth.insert(key.into(), value);
                }
            }
            if !oauth.is_empty() {
                entry.insert("oauth".into(), Value::Object(oauth));
            }
            if let Some(provider) = &options.auth_provider {
                entry.insert("auth".into(), json!({ "provider": provider }));
            }
        }
        (None, Some((command, args))) => {
            if !options.headers.is_empty() || options.bearer_token_env_var.is_some() {
                bail!("headers apply to HTTP servers; give --url");
            }
            entry.insert("command".into(), json!(command));
            if !args.is_empty() {
                entry.insert("args".into(), json!(args));
            }
            let env = pairs(&options.env, "environment variable")?;
            if !env.is_empty() {
                entry.insert("env".into(), Value::Object(env));
            }
            if let Some(cwd) = &options.cwd {
                entry.insert("cwd".into(), json!(cwd));
            }
        }
    }
    if let Some(description) = &options.description {
        entry.insert("description".into(), json!(description));
    }
    if let Some(exposure) = &options.exposure {
        entry.insert("exposure".into(), json!(exposure));
    }
    Ok(Value::Object(entry))
}

pub async fn add(root: &Path, name: &str, local: bool, options: &AddOptions) -> Result<()> {
    let entry = entry_for(options)?;
    config::parse_server(name, &entry).map_err(anyhow::Error::msg)?;
    if local && options.auth_provider.is_some() {
        bail!("auth is only allowed in the global {}", config::FILE_NAME);
    }
    let path = file_for(root, local)?;
    let replaced = config::add_server(&path, name, entry).map_err(anyhow::Error::msg)?;
    match replaced {
        true => println!("Replaced MCP server {name} in {}.", path.display()),
        false => println!("Added MCP server {name} to {}.", path.display()),
    }
    if local && !trusted(root).await {
        println!("This project is not trusted yet, so sessions here will not read it until it is.");
    }
    Ok(())
}

pub async fn remove(root: &Path, name: &str, local: bool) -> Result<()> {
    let path = file_for(root, local)?;
    match config::remove_server(&path, name).map_err(anyhow::Error::msg)? {
        true => println!("Removed MCP server {name} from {}.", path.display()),
        false => bail!("{} does not define MCP server {name}", path.display()),
    }
    Ok(())
}

/// Connect every server and say how each one went. Fails when a configuration entry is invalid or
/// a server that is turned on did not connect.
pub async fn list(root: &Path, as_json: bool) -> Result<()> {
    let trusted = trusted(root).await;
    let servers = servers(root, config::load(root, trusted))?;
    servers.connect_all().await;
    let reports = servers.report();
    let failed = !servers.errors().is_empty()
        || reports
            .iter()
            .any(|report| report.enabled && !matches!(report.status, Status::Connected { .. }));

    if as_json {
        let listed: Vec<Value> = reports
            .iter()
            .map(|report| {
                json!({
                    "name": report.name,
                    "scope": report.scope.name(),
                    "source": report.source,
                    "target": report.target,
                    "enabled": report.enabled,
                    "exposure": report.exposure.map(micro_mcp::Exposure::name),
                    "description": report.description,
                    "state": report.status.label(),
                    "tools": report.tools,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "servers": listed, "errors": servers.errors() })
            )?
        );
    } else {
        if reports.is_empty() && servers.errors().is_empty() {
            println!(
                "No MCP servers. Add one with `micro mcp add <name> -- <command>` or \
                 `micro mcp add <name> --url <url>`."
            );
        }
        for report in &reports {
            println!("{}  {}", report.name, report.status.label());
            println!("  {} ({})", report.target, report.scope.name());
            if let Status::Failed(error) = &report.status {
                for line in error.lines().skip(1) {
                    println!("  {line}");
                }
            }
            for tool in &report.tools {
                println!("  - {tool}");
            }
        }
        for error in servers.errors() {
            eprintln!("error: {error}");
        }
        if !trusted && config::project_path(root).exists() {
            eprintln!(
                "note: {} was not read because this project is not trusted",
                config::project_path(root).display()
            );
        }
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

/// A URL a terminal shows as a link, written so it stays one when it wraps.
fn hyperlink(url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
}

pub async fn login(root: &Path, name: &str, timeout: Option<u64>) -> Result<()> {
    let trusted = trusted(root).await;
    let servers = servers(root, config::load(root, trusted))?;
    if servers.entry(name).is_none() {
        bail!("no MCP server is called {name}");
    }
    let _ = servers.connect(name).await;

    let pending = match servers
        .begin_sign_in(name)
        .await
        .map_err(anyhow::Error::msg)?
    {
        SignIn::Authorized => {
            println!("Signed in to {name}.");
            return Ok(());
        }
        SignIn::Pending(pending) => pending,
    };
    let url = pending.authorization_url.to_string();
    let shown = match std::io::stdout().is_terminal() {
        true => hyperlink(&url),
        false => url.clone(),
    };
    println!("Opening the sign-in page for {name}. If it does not open, visit:\n{shown}");
    println!("If the browser runs on another machine, paste the address it was sent back to here.");
    micro_mcp::oauth::open_browser(&url);

    let within = timeout.map_or(SIGN_IN_TIMEOUT, Duration::from_secs);
    let pasted = async {
        use tokio::io::AsyncBufReadExt as _;
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) if !line.trim().is_empty() => return Some(line),
                Ok(Some(_)) => continue,
                _ => std::future::pending::<()>().await,
            }
        }
    };

    let mut pending = pending;
    let outcome = tokio::select! {
        finished = pending.wait_for_browser(within) => match finished {
            Ok(callback) => pending.complete(callback).await,
            Err(error) => Err(error),
        },
        Some(line) = pasted => match pending.parse_redirect(&line) {
            Ok(callback) => pending.complete(callback).await,
            Err(error) => Err(error),
        },
    };
    outcome.map_err(|error| anyhow::anyhow!("signing in to {name} failed: {error}"))?;
    println!("Signed in to {name}.");
    Ok(())
}

pub async fn logout(root: &Path, name: &str) -> Result<()> {
    let trusted = trusted(root).await;
    let servers = servers(root, config::load(root, trusted))?;
    match servers.sign_out(name).map_err(anyhow::Error::msg)? {
        true => println!("Signed out of {name}."),
        false => println!("{name} had no stored credentials."),
    }
    Ok(())
}

/// What `/mcp` shows: every server, one to look at, or what can be done to one.
pub fn command(servers: &Servers, argument: &str) -> CommandOutcome {
    let mut words = argument.split_whitespace();
    let (first, second) = (words.next(), words.next());
    match (first, second) {
        (None, _) => overview(servers),
        (Some("login" | "logout" | "reconnect"), Some(_)) => CommandOutcome::Mcp {
            argument: argument.to_string(),
        },
        (Some(action @ ("login" | "logout" | "reconnect")), None) => {
            CommandOutcome::error(format!("Usage: /mcp {action} <server>"))
        }
        (Some(name), None) => match servers
            .report()
            .into_iter()
            .find(|report| report.name == name)
        {
            Some(report) => actions(&report),
            None => CommandOutcome::error(format!("no MCP server is called {name}")),
        },
        (Some(_), Some(_)) => {
            CommandOutcome::error("Usage: /mcp [server] | /mcp login|logout|reconnect <server>")
        }
    }
}

fn overview(servers: &Servers) -> CommandOutcome {
    let reports = servers.report();
    if reports.is_empty() {
        let mut text = "No MCP servers. Add one with `micro mcp add`, or in mcp.json.".to_string();
        for error in servers.errors() {
            text.push_str(&format!("\n{error}"));
        }
        return CommandOutcome::info(text);
    }
    CommandOutcome::Choose(
        Picker::new(
            "MCP servers",
            reports
                .iter()
                .map(|report| {
                    PickerItem::new(
                        report.name.clone(),
                        report.status.label(),
                        format!("/mcp {}", report.name),
                    )
                })
                .collect(),
        )
        .searchable(),
    )
}

fn actions(report: &micro_mcp::ServerReport) -> CommandOutcome {
    let name = &report.name;
    let mut items = Vec::new();
    if report.signs_in {
        items.push(PickerItem::new(
            "Sign in",
            "authorize micro in the browser",
            format!("/mcp login {name}"),
        ));
    }
    if report.signed_in {
        items.push(PickerItem::new(
            "Sign out",
            "forget the stored credentials",
            format!("/mcp logout {name}"),
        ));
    }
    if report.enabled {
        items.push(PickerItem::new(
            "Reconnect",
            "connect again and list its tools",
            format!("/mcp reconnect {name}"),
        ));
    }
    let mut detail = vec![
        report.status.label(),
        format!("{} ({})", report.target, report.scope.name()),
    ];
    if let Status::Failed(error) = &report.status {
        detail.extend(error.lines().skip(1).map(str::to_string));
    }
    if let Some(description) = &report.description {
        detail.push(description.clone());
    }
    detail.extend(report.tools.iter().map(|tool| format!("- {tool}")));
    if items.is_empty() {
        return CommandOutcome::info(format!("{name}\n{}", detail.join("\n")));
    }
    CommandOutcome::Choose(Picker::new(format!("{name}: {}", detail.join(" · ")), items).titled())
}

/// Carry out a `/mcp` action that needs the network.
pub async fn apply(
    servers: &Servers,
    argument: &str,
    notifier: Option<&micro_tui::UiAsker>,
) -> Applied {
    let mut words = argument.split_whitespace();
    let (Some(action), Some(name)) = (words.next(), words.next()) else {
        return Applied::error("Usage: /mcp login|logout|reconnect <server>");
    };
    match action {
        "reconnect" => match servers.reconnect(name).await {
            Ok(count) => Applied::note(format!(
                "Reconnected {name}: {count} tools, found through tool_search."
            )),
            Err(error) => Applied::error(error.to_string()),
        },
        "logout" => match servers.sign_out(name) {
            Ok(true) => Applied::note(format!("Signed out of {name}.")),
            Ok(false) => Applied::note(format!("{name} had no stored credentials.")),
            Err(error) => Applied::error(error),
        },
        "login" => sign_in(servers, name, notifier).await,
        other => Applied::error(format!("unknown /mcp action `{other}`")),
    }
}

async fn sign_in(servers: &Servers, name: &str, notifier: Option<&micro_tui::UiAsker>) -> Applied {
    let pending = match servers.begin_sign_in(name).await {
        Ok(SignIn::Authorized) => {
            return match servers.reconnect(name).await {
                Ok(count) => Applied::note(format!("Signed in to {name}: {count} tools.")),
                Err(error) => Applied::error(error.to_string()),
            }
        }
        Ok(SignIn::Pending(pending)) => pending,
        Err(error) => return Applied::error(format!("Cannot sign in to {name}: {error}")),
    };

    let url = pending.authorization_url.to_string();
    micro_mcp::oauth::open_browser(&url);

    let servers = servers.clone();
    let notifier = notifier.cloned();
    let name = name.to_string();
    let announced = format!(
        "Opening the sign-in page for {name}. If it does not open, visit:\n{url}\n\
         Cmd/Ctrl+click the link to open it. The session picks up the sign-in once the browser \
         comes back."
    );
    tokio::spawn(async move {
        let said = match pending.finish(SIGN_IN_TIMEOUT).await {
            Ok(()) => match servers.reconnect(&name).await {
                Ok(count) => {
                    format!("Signed in to {name}: {count} tools, found through tool_search.")
                }
                Err(error) => format!("Signed in to {name}, but it did not connect: {error}"),
            },
            Err(error) => format!("Signing in to {name} failed: {error}"),
        };
        if let Some(notifier) = notifier {
            notifier.ask("notify", said, None, Vec::new()).await;
        }
    });
    Applied::note(announced)
}

/// The `mcp_servers` system prompt section as the servers stand when a run starts, so a server
/// that connected since, and said what it offers, or that an extension added, is listed with it.
pub struct ServersSection {
    servers: Servers,
    /// The servers whose tools were not declared to the model when the session started.
    undeclared: Vec<String>,
    codemode: bool,
}

impl ServersSection {
    pub fn new(servers: Servers, undeclared: Vec<String>, codemode: bool) -> Self {
        ServersSection {
            servers,
            undeclared,
            codemode,
        }
    }
}

impl micro_agent::LiveSection for ServersSection {
    fn name(&self) -> &str {
        "mcp_servers"
    }

    fn render(&self) -> Option<String> {
        let mut undeclared = self.undeclared.clone();
        for entry in self.servers.entries() {
            if entry.config.has_undeclared_tools() && !undeclared.contains(&entry.name) {
                undeclared.push(entry.name);
            }
        }
        self.servers.prompt_section(&undeclared, self.codemode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_after_the_options_is_a_stdio_server() {
        let entry = entry_for(&AddOptions {
            env: vec!["API_KEY=${TOOLS_KEY}".into()],
            description: Some("Project tools".into()),
            command: vec!["uvx".into(), "tools-mcp".into(), "--verbose".into()],
            ..AddOptions::default()
        })
        .unwrap();
        assert_eq!(
            entry,
            json!({
                "command": "uvx",
                "args": ["tools-mcp", "--verbose"],
                "env": { "API_KEY": "${TOOLS_KEY}" },
                "description": "Project tools",
            })
        );
    }

    #[test]
    fn a_url_is_an_http_server_with_its_credentials() {
        let entry = entry_for(&AddOptions {
            url: Some("https://example.com/mcp".into()),
            bearer_token_env_var: Some("DOCS_TOKEN".into()),
            oauth_client_name: Some("Claude Code".into()),
            ..AddOptions::default()
        })
        .unwrap();
        assert_eq!(entry["headers"]["Authorization"], "Bearer ${DOCS_TOKEN}");
        assert_eq!(entry["oauth"]["clientName"], "Claude Code");

        assert!(entry_for(&AddOptions::default()).is_err());
        assert!(entry_for(&AddOptions {
            url: Some("https://x".into()),
            command: vec!["x".into()],
            ..AddOptions::default()
        })
        .is_err());
    }
}
