//! The non-conversational commands: credentials, the model catalog, and saved sessions.

use anyhow::bail;
use anyhow::Context as _;
use anyhow::Result;
use micro_auth::AuthStore;
use micro_auth::Credential;
use micro_auth::LoginFlow;
use micro_models::Catalog;
use micro_session::SessionStore;
use std::io::BufRead as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

pub async fn auth_status() -> Result<()> {
    let store = AuthStore::open()?;
    let listed = store.status();

    let width = listed
        .iter()
        .map(|status| status.provider.chars().count())
        .max()
        .unwrap_or(0);

    for status in listed {
        let blank = store
            .get(&status.provider)
            .is_some_and(|credential| credential.token().trim().is_empty());

        let state = if !status.is_authenticated() {
            "not configured"
        } else if blank {
            "empty"
        } else if status.needs_refresh {
            "expired"
        } else {
            "ready"
        };
        let source = match &status.source {
            micro_auth::CredentialSource::Stored => "stored".to_string(),
            micro_auth::CredentialSource::Environment { variable } => format!("${variable}"),
            micro_auth::CredentialSource::Federation => "workload identity federation".to_string(),
            micro_auth::CredentialSource::Missing => String::new(),
        };
        println!("{:<width$}  {:<14} {source}", status.provider, state);
    }
    Ok(())
}

pub async fn auth_login(provider: &str, method: Option<&str>) -> Result<()> {
    let store = AuthStore::open()?;
    let mut method = method.map(str::to_string);
    loop {
        let options = login_options(provider, method.as_deref())?;
        match store
            .begin_login_with(provider, method.as_deref(), &options)
            .await?
        {
            LoginFlow::Choose { title, options, .. } => {
                method = Some(choose(&title, &options)?);
            }
            LoginFlow::ApiKey {
                provider,
                env_names,
            } => {
                if !env_names.is_empty() {
                    println!("Or set one of: {}", env_names.join(", "));
                }
                print!("Paste your {provider} API key: ");
                std::io::stdout().flush()?;
                let mut key = String::new();
                std::io::stdin().lock().read_line(&mut key)?;
                let key = key.trim();
                if key.is_empty() {
                    bail!("no key entered");
                }
                store.store_api_key(&provider, key)?;
                println!("Stored a credential for {provider}.");
                return Ok(());
            }
            LoginFlow::DeviceCode(pending) => {
                println!("Open {}", pending.verification_uri());
                println!("Enter the code: {}", pending.user_code());
                println!("Waiting for authorization…");
                store.complete_device_login(&pending).await?;
                println!("Signed in to {}.", pending.provider);
                return Ok(());
            }
            LoginFlow::Browser(pending) => {
                if let Some(note) = pending.note() {
                    println!("{note}");
                }
                match micro_auth::oauth::open_browser(pending.url()) {
                    true => println!("Opened your browser at:\n{}", pending.url()),
                    false => println!("Open this page to sign in:\n{}", pending.url()),
                }
                println!("{}", pending.instructions());
                print!("{} ", pending.prompt());
                std::io::stdout().flush()?;
                store
                    .complete_browser_login(&pending, read_pasted_line())
                    .await?;
                println!();
                println!("Signed in to {}.", pending.provider);
                return Ok(());
            }
        }
    }
}

/// Sign in with ChatGPT registers this installation, so it is the one login that needs its id.
fn login_options(provider: &str, method: Option<&str>) -> Result<micro_auth::LoginOptions> {
    let wants_device_id = micro_auth::canonical_provider(provider) == micro_auth::OPENAI
        && method.is_some_and(|method| method != micro_auth::METHOD_API_KEY);
    if !wants_device_id {
        return Ok(micro_auth::LoginOptions::default());
    }
    let device_id = micro_config::Config::device_id(micro_auth::oauth::random_uuid)
        .context("could not record this installation's device id")?;
    Ok(micro_auth::LoginOptions {
        device_id: Some(device_id),
    })
}

/// Ask which option to take, by number; an empty answer takes the first.
fn choose(title: &str, options: &[micro_auth::LoginOption]) -> Result<String> {
    println!("{title}");
    for (index, option) in options.iter().enumerate() {
        println!("  {}. {}", index + 1, option.label);
    }
    print!("Choose [1]: ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    pick(answer.trim(), options)
        .map(str::to_string)
        .with_context(|| format!("\"{}\" is not one of the options", answer.trim()))
}

/// The option an answer names, by number or by id.
fn pick<'a>(answer: &str, options: &'a [micro_auth::LoginOption]) -> Option<&'a str> {
    if answer.is_empty() {
        return options.first().map(|option| option.id.as_str());
    }
    if let Ok(number) = answer.parse::<usize>() {
        return number
            .checked_sub(1)
            .and_then(|index| options.get(index))
            .map(|option| option.id.as_str());
    }
    options
        .iter()
        .find(|option| option.id.eq_ignore_ascii_case(answer))
        .map(|option| option.id.as_str())
}

/// One line from the terminal, or nothing at the end of input, read off the async runtime so the
/// browser can finish the sign-in meanwhile. The read runs on a thread of its own, which the process
/// does not wait for when the browser finishes first.
async fn read_pasted_line() -> Option<String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim().to_string()).filter(|line| !line.is_empty()),
        };
        let _ = sender.send(read);
    });
    receiver.await.ok().flatten()
}

/// What an auth command is about, as given on the command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthTarget {
    /// A provider, or a model whose provider is meant.
    pub name: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// The provider an auth command is about.
fn resolve_target(target: &AuthTarget, catalog: &Catalog, store: &AuthStore) -> Result<String> {
    let known = |name: &str| {
        micro_auth::provider_entry(name)
            .map(|entry| entry.id.clone())
            .or_else(|| {
                let canonical = micro_auth::canonical_provider(name);
                catalog
                    .models()
                    .iter()
                    .any(|model| model.provider == canonical)
                    .then(|| canonical.to_string())
            })
    };

    let (provider, model) = match (&target.provider, &target.model, &target.name) {
        (Some(provider), model, name) => (Some(provider.clone()), model.clone().or(name.clone())),
        (None, Some(model), _) => (None, Some(model.clone())),
        (None, None, Some(name)) if known(name).is_some() => (Some(name.clone()), None),
        (None, None, Some(name)) => (None, Some(name.clone())),
        (None, None, None) => {
            bail!("name a provider or a model, such as `micro auth check anthropic`")
        }
    };

    if let Some(provider) = &provider {
        let Some(id) = known(provider) else {
            bail!("unknown provider \"{provider}\"");
        };
        if let Some(model) = &model {
            let serves = catalog
                .resolve(&format!("{id}/{model}"))
                .candidates()
                .iter()
                .any(|candidate| candidate.provider == id);
            if !serves {
                bail!("{id} has no model \"{model}\"");
            }
        }
        return Ok(id);
    }

    let model = model.unwrap_or_default();
    let mut providers: Vec<String> = catalog
        .resolve(&model)
        .candidates()
        .iter()
        .map(|candidate| candidate.provider.clone())
        .collect();
    providers.dedup();
    if providers.is_empty() {
        bail!("no model matches \"{model}\"; `micro models` lists them");
    }
    if providers.len() > 1 {
        let configured: Vec<String> = providers
            .iter()
            .filter(|provider| store.status_of(provider).is_authenticated())
            .cloned()
            .collect();
        if configured.len() == 1 {
            return Ok(configured[0].clone());
        }
        bail!(
            "several providers serve \"{model}\" ({}); name one with --provider",
            providers.join(", ")
        );
    }
    Ok(providers.remove(0))
}

/// How `micro auth check` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckOptions {
    pub json: bool,
    pub credentials: bool,
    pub refresh: bool,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckResult {
    status: &'static str,
    provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credentials: Option<String>,
}

impl CheckResult {
    fn not_ready(provider: String, reason: &'static str) -> Self {
        CheckResult {
            status: "not_ready",
            provider,
            reason: Some(reason),
            auth_type: None,
            credentials: None,
        }
    }

    fn invalid(provider: String) -> Self {
        CheckResult {
            status: "invalid",
            provider,
            reason: Some("invalid_state"),
            auth_type: None,
            credentials: None,
        }
    }

    /// `0` when ready, `1` when not, `2` when the check itself could not be made.
    fn exit_code(&self) -> i32 {
        match self.status {
            "ready" => 0,
            "not_ready" => 1,
            _ => 2,
        }
    }

    fn render(&self, json: bool) -> String {
        match (json, &self.credentials) {
            (true, _) => serde_json::to_string(self).unwrap_or_default(),
            (false, Some(credential)) => credential.clone(),
            (false, None) => self.status.to_string(),
        }
    }
}

/// Check that a credential resolves, print the outcome, and return the exit code.
pub async fn auth_check(target: &AuthTarget, options: CheckOptions) -> i32 {
    let fallback = target
        .provider
        .clone()
        .or(target.model.clone())
        .or(target.name.clone())
        .unwrap_or_default();
    let result = match AuthStore::open() {
        Ok(store) => {
            let catalog = Catalog::load().unwrap_or_else(|_| Catalog::bundled());
            match resolve_target(target, &catalog, &store) {
                Ok(provider) => check(&store, provider, options).await,
                Err(error) if error.to_string().starts_with("unknown provider") => {
                    CheckResult::not_ready(fallback, "provider_not_found")
                }
                Err(error) => {
                    eprintln!("Error: {error}");
                    return 2;
                }
            }
        }
        Err(_) => CheckResult::invalid(fallback),
    };
    println!("{}", result.render(options.json));
    result.exit_code()
}

async fn check(store: &AuthStore, provider: String, options: CheckOptions) -> CheckResult {
    let status = store.status_of(&provider);
    if !status.is_authenticated() {
        return CheckResult::not_ready(provider, "credentials_not_configured");
    }
    let auth_type = match (&status.source, status.method) {
        (micro_auth::CredentialSource::Stored, micro_auth::AuthMethod::OAuth) => "oauth",
        _ => "api_key",
    };

    let credential = match (options.refresh, store.get(&provider)) {
        (false, Some(stored)) => Some(stored.token().to_string()),
        (false, None) if !options.credentials => None,
        _ => match store.resolve(&provider).await {
            Ok(resolved) => Some(resolved.token().to_string()),
            Err(_) => return CheckResult::invalid(provider),
        },
    };

    let credentials = match options.credentials {
        false => None,
        true => match credential.filter(|token| !token.trim().is_empty()) {
            Some(token) => Some(token),
            None => return CheckResult::not_ready(provider, "credential_not_available"),
        },
    };
    CheckResult {
        status: "ready",
        provider,
        reason: None,
        auth_type: Some(auth_type),
        credentials,
    }
}

/// Print the API key a provider resolves to.
pub async fn auth_print_api_key(target: &AuthTarget) -> Result<()> {
    let store = AuthStore::open()?;
    let catalog = Catalog::load().unwrap_or_else(|_| Catalog::bundled());
    let provider = resolve_target(target, &catalog, &store)?;
    if matches!(store.get(&provider), Some(Credential::OAuth(_))) {
        bail!("{provider} is configured with OAuth, not an API key; use print-bearer-token");
    }
    match store.resolve(&provider).await? {
        Credential::ApiKey { key } if !key.trim().is_empty() => println!("{key}"),
        _ => bail!("{provider} has no usable API key"),
    }
    Ok(())
}

/// A bearer token is refreshed unless it stays valid this long.
const DEFAULT_MIN_EXPIRY_MS: i64 = 30 * 60 * 1000;

/// Print a provider's OAuth bearer token, refreshed first when it would lapse within `min_expiry`.
pub async fn auth_print_bearer_token(target: &AuthTarget, min_expiry: Option<&str>) -> Result<()> {
    let min_validity_ms = match min_expiry {
        Some(duration) => parse_duration_ms(duration).with_context(|| {
            format!("--min-expiry must be a duration such as 30m or 1h, not \"{duration}\"")
        })?,
        None => DEFAULT_MIN_EXPIRY_MS,
    };
    let store = AuthStore::open()?;
    let catalog = Catalog::load().unwrap_or_else(|_| Catalog::bundled());
    let provider = resolve_target(target, &catalog, &store)?;
    match store.resolve_valid_for(&provider, min_validity_ms).await? {
        Credential::OAuth(oauth) if !oauth.access_token.trim().is_empty() => {
            println!("{}", oauth.access_token)
        }
        _ => bail!("{provider} is not configured with an OAuth bearer token"),
    }
    Ok(())
}

/// `500ms`, `30s`, `30m` or `1h`, in milliseconds.
fn parse_duration_ms(text: &str) -> Option<i64> {
    let text = text.trim().to_ascii_lowercase();
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (amount, unit) = text.split_at(split);
    let amount: i64 = amount.parse().ok()?;
    let scale = match unit {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        _ => return None,
    };
    amount.checked_mul(scale)
}

pub async fn auth_logout(provider: &str) -> Result<()> {
    AuthStore::open()?.remove(provider)?;
    println!("Removed the stored credential for {provider}.");
    Ok(())
}

pub async fn models(query: Option<&str>, live: bool, kind: &str) -> Result<()> {
    let Some(kind) = micro_models::ModelType::parse(kind) else {
        bail!("there are chat, image and classifier models, not {kind} models");
    };
    let mut catalog = Catalog::load().unwrap_or_else(|_| Catalog::bundled());
    let store = AuthStore::open()?;
    if live {
        crate::runtime::merge_live_listings(&mut catalog, &store).await;
    }
    crate::runtime::merge_llama_cpp(&mut catalog, &store).await;

    let models = match (query, kind) {
        (Some(query), micro_models::ModelType::Chat) => catalog.resolve(query).candidates(),
        (Some(query), kind) => catalog
            .models_of_type(kind)
            .into_iter()
            .filter(|model| model.qualified_id().contains(query))
            .collect(),
        (None, kind) => catalog.models_of_type(kind),
    };

    if models.is_empty() {
        println!("No models match.");
        return Ok(());
    }

    for model in models {
        println!(
            "{:<44} {:>9} in  {:>9} out  {:>9} ctx",
            model.qualified_id(),
            format!("${:.2}", model.cost.input),
            format!("${:.2}", model.cost.output),
            model.context_window
        );
    }
    Ok(())
}

pub async fn sessions_list(workspace: &std::path::Path, all: bool) -> Result<()> {
    let store = SessionStore::from_env()?;
    let sessions = match all {
        true => store.list().await?,
        false => store.list_in(workspace).await?,
    };

    if sessions.is_empty() {
        println!("No sessions yet.");
        return Ok(());
    }

    for meta in sessions {
        println!("{:<22} {:<28} {}", meta.id, meta.model_id, meta.title);
    }
    Ok(())
}

pub async fn sessions_show(id: &str, turn: Option<u64>, raw: bool) -> Result<()> {
    let store = SessionStore::from_env()?;
    let loaded = store.load(id).await?;
    let turns = recorded_turns(&loaded.session);

    let Some(wanted) = turn.or_else(|| turns.last().map(|last| last.turn)) else {
        println!(
            "{id}  {}  {}",
            loaded.session.meta().model_id,
            loaded.session.meta().workspace.display()
        );
        println!("No recorded turns.");
        return Ok(());
    };

    if turn.is_none() && !raw {
        println!(
            "{id}  {}  {}",
            loaded.session.meta().model_id,
            loaded.session.meta().workspace.display()
        );
        for recorded in &turns {
            println!(
                "turn {:<4} {:<28} prefix {}  {} in  {} out  {} cached",
                recorded.turn,
                format!("{}/{}", recorded.provider, recorded.model),
                short(&recorded.prefix_hash),
                recorded.usage.input,
                recorded.usage.output,
                recorded.usage.cache_read,
            );
        }
        return Ok(());
    }

    let rebuilt = store.reconstruct_turn(id, wanted).await?;
    match raw {
        true => print_request(id, &rebuilt),
        false => {
            print_turn(id, &rebuilt);
            Ok(())
        }
    }
}

/// One turn as the ledger describes it, without rebuilding what it sent.
struct RecordedTurn {
    turn: u64,
    provider: String,
    model: String,
    prefix_hash: String,
    usage: micro_types::Usage,
}

pub async fn sessions_export(id: &str) -> Result<()> {
    let raw = SessionStore::from_env()?
        .raw_log(id)
        .await
        .with_context(|| format!("cannot read the log of session {id}"))?;

    let mut skipped = 0;
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(_) => println!("{line}"),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        eprintln!("note: skipped {skipped} unreadable line(s) in session {id}");
    }
    Ok(())
}

pub async fn bill(id: &str, diff: Option<u64>) -> Result<()> {
    let store = SessionStore::from_env()?;
    let catalog = Catalog::load().unwrap_or_else(|_| Catalog::bundled());
    let billed = micro_commands::bill(&store, &catalog, id)
        .await
        .map_err(|reason| anyhow::anyhow!(reason))?;

    let report = match diff {
        Some(turn) => billed
            .added_by(turn)
            .map_err(|reason| anyhow::anyhow!(reason))?,
        None => billed.report(),
    };
    println!("{report}");
    Ok(())
}

pub async fn why_miss(id: &str, turn: u64) -> Result<()> {
    let store = SessionStore::from_env()?;
    let explanation = micro_commands::why_miss(&store, id, Some(turn))
        .await
        .map_err(|reason| anyhow::anyhow!(reason))?;
    println!("{explanation}");
    Ok(())
}

/// Every turn the session recorded a request for, in order and without repeats.
fn recorded_turns(session: &micro_session::Session) -> Vec<RecordedTurn> {
    let mut turns: Vec<RecordedTurn> = Vec::new();
    for recorded in session.events() {
        match &recorded.event {
            micro_types::LedgerEvent::TurnRequest {
                turn,
                provider,
                model,
                prefix_hash,
                ..
            } => {
                let described = RecordedTurn {
                    turn: *turn,
                    provider: provider.clone(),
                    model: model.clone(),
                    prefix_hash: prefix_hash.clone(),
                    usage: micro_types::Usage::default(),
                };
                match turns.last_mut().filter(|last| last.turn == *turn) {
                    Some(last) => *last = described,
                    None => turns.push(described),
                }
            }
            micro_types::LedgerEvent::TurnUsage { turn, usage, .. } => {
                if let Some(found) = turns.iter_mut().find(|recorded| recorded.turn == *turn) {
                    found.usage = *usage;
                }
            }
            _ => {}
        }
    }
    turns
}

/// What the model was shown at one turn, with every stretch of the prompt attributed.
fn print_turn(id: &str, turn: &micro_session::ReconstructedTurn) {
    println!(
        "turn {} of session {id}  {}/{}  attempt {}",
        turn.turn, turn.provider, turn.model_id, turn.attempt
    );

    let prompt = turn.system_prompt.as_deref().unwrap_or_default();
    println!(
        "\nsystem prompt  {} bytes  prefix {}",
        prompt.len(),
        short(&turn.prefix_hash)
    );
    for span in &turn.prefix_spans {
        println!(
            "  {:<24} {:>7} bytes  {}",
            span.source,
            span.bytes,
            short(&span.hash)
        );
    }

    let tools: Vec<&str> = turn.tools.iter().map(|tool| tool.name.as_str()).collect();
    println!("\ntools  {}", tools.join(", "));

    println!("\nmessages ({})", turn.messages.len());
    for (index, message) in turn.messages.iter().enumerate() {
        let named = match turn.message_entry_ids.len() == turn.messages.len() {
            true => turn.message_entry_ids[index].clone(),
            false => "-".to_string(),
        };
        let said: String = message
            .content()
            .iter()
            .map(micro_types::ContentBlock::as_text)
            .collect();
        println!("  {named:<4} {:<12} {}", role_of(message), one_line(&said));
    }

    match turn.usage {
        Some(usage) => println!(
            "\nusage  {} in  {} out  {} cache read  {} cache write",
            usage.input, usage.output, usage.cache_read, usage.cache_write
        ),
        None => println!("\nusage  not recorded; the turn did not come back"),
    }
    println!("request  {}", turn.request_hash);
}

/// The request as it went out.
fn print_request(id: &str, turn: &micro_session::ReconstructedTurn) -> Result<()> {
    if let Some(body) = &turn.recorded_request_body {
        let hash = micro_types::content_hash(body);
        if hash != turn.request_hash {
            anyhow::bail!(
                "stored request body for session {id} turn {} failed verification: expected {}, got {}",
                turn.turn,
                turn.request_hash,
                hash
            );
        }
        serde_json::from_slice::<serde_json::Value>(body)?;
        std::io::stdout().write_all(body)?;
        if !body.ends_with(b"\n") {
            println!();
        }
        return Ok(());
    }

    let catalog = Catalog::load().unwrap_or_else(|_| Catalog::bundled());
    let model = catalog.get(&turn.provider, &turn.model_id).ok_or_else(|| {
        anyhow::anyhow!(
            "{}/{} is not in the catalog any more, so its request shape is not known",
            turn.provider,
            turn.model_id
        )
    })?;

    let context = micro_types::Context {
        system_prompt: turn.system_prompt.clone(),
        messages: turn.messages.clone(),
        tools: turn.tools.clone(),
        headers: Vec::new(),

        cache_key: Some(id.to_string()),
    };
    let payload = micro_provider::client_for_model(model).payload(&turn.model, &context);
    let body = serde_json::to_vec(&payload)?;

    if micro_types::content_hash(&body) != turn.request_hash {
        anyhow::bail!(
            "request reconstruction failed verification for session {id} turn {} (expected {})",
            turn.turn,
            short(&turn.request_hash),
        );
    }
    std::io::stdout().write_all(&body)?;
    if !body.ends_with(b"\n") {
        println!();
    }
    Ok(())
}

/// A hash short enough to read, which is all a person comparing two of them needs.
fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn role_of(message: &micro_types::Message) -> &'static str {
    match message {
        micro_types::Message::User { .. } => "user",
        micro_types::Message::Assistant(_) => "assistant",
        micro_types::Message::ToolResult { .. } => "tool result",
    }
}

/// Text flattened to one line short enough to sit in a column.
fn one_line(text: &str) -> String {
    let single = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match single.chars().count() > 60 {
        true => format!("{}…", single.chars().take(60).collect::<String>()),
        false => single,
    }
}

pub async fn sessions_delete(id: &str) -> Result<()> {
    SessionStore::from_env()?
        .delete(id)
        .await
        .with_context(|| format!("cannot delete session {id}"))?;
    println!("Deleted session {id}.");
    Ok(())
}

/// The id of the most recent session for this workspace, for `--continue`.
pub async fn latest_session(workspace: &std::path::Path) -> Result<String> {
    let store = SessionStore::from_env()?;
    store
        .list_in(workspace)
        .await?
        .into_iter()
        .next()
        .map(|meta| meta.id)
        .ok_or_else(|| anyhow::anyhow!("no session to continue in this workspace"))
}

/// Where a user install of an extension package goes: micro's data directory.
fn install_root() -> Result<PathBuf> {
    micro_dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("no home directory; set {}", micro_dirs::MICRO_DIR_ENV))
}

pub async fn install(source: &str, local: bool, workspace: &Path) -> Result<()> {
    let parsed = micro_extensions::Source::parse(source).map_err(anyhow::Error::msg)?;
    let home = install_root()?;

    println!("Installing {}...", parsed.canonical());
    let installed = micro_extensions::install(&parsed, &home, workspace, local)
        .await
        .map_err(anyhow::Error::msg)?;

    remember(&installed.source, true)?;
    println!(
        "Installed {} to {}",
        installed.source,
        installed.path.display()
    );

    let workspace = std::env::current_dir().unwrap_or_default();
    let entries = match installed.path.is_dir() {
        true => micro_extensions::entries_of(&installed.path)
            .unwrap_or_else(|| micro_extensions::in_directory(&installed.path)),
        false => vec![installed.path.clone()],
    };
    match micro_extensions::Host::start(&home, &entries, &workspace, false, false, "print").await {
        Ok(host) => {
            for extension in &host.loaded().extensions {
                for tool in &extension.tools {
                    println!("  tool     {}", tool.name);
                }
                for command in &extension.commands {
                    println!("  command  /{}", command.name);
                }
            }
            for failure in &host.loaded().errors {
                println!(
                    "  warning  {} did not load: {}",
                    failure.path, failure.error
                );
            }
            host.shutdown("quit").await;
        }
        Err(error) => println!("  note     {error}"),
    }
    Ok(())
}

pub async fn remove(source: &str, local: bool, workspace: &Path) -> Result<()> {
    let parsed = micro_extensions::Source::parse(source).map_err(anyhow::Error::msg)?;
    let home = install_root()?;

    deactivate(
        &parsed.install_path(&home, workspace, local),
        &home,
        workspace,
    )
    .await;
    micro_extensions::remove(&parsed, &home, workspace, local).map_err(anyhow::Error::msg)?;
    let forgotten = remember(&parsed.canonical(), false)?;

    match forgotten {
        true => println!("Removed {}.", parsed.canonical()),
        false => println!("{} was not installed.", parsed.canonical()),
    }
    Ok(())
}

/// Let a package's extensions go before its files do.
async fn deactivate(path: &Path, home: &Path, workspace: &Path) {
    if !path.exists() {
        return;
    }
    let entries = match path.is_dir() {
        true => micro_extensions::entries_of(path)
            .unwrap_or_else(|| micro_extensions::in_directory(path)),
        false => vec![path.to_path_buf()],
    };
    if entries.is_empty() {
        return;
    }
    let Ok(host) =
        micro_extensions::Host::start(home, &entries, workspace, false, false, "print").await
    else {
        return;
    };
    for extension in &host.loaded().extensions {
        if let Err(error) = host.deactivate(&extension.path).await {
            println!(
                "  note     {} did not deactivate cleanly: {error}",
                extension.path
            );
        }
    }
    host.shutdown("quit").await;
}

pub async fn list_packages() -> Result<()> {
    let path = micro_config::default_path()?;
    let config = micro_config::Config::load_from(&path)?;
    let sources = config.extensions.clone().unwrap_or_default();

    if sources.is_empty() {
        println!("No extension packages installed.");
        return Ok(());
    }

    let home = install_root()?;
    let workspace = std::env::current_dir().unwrap_or_default();

    let installed: Vec<(String, PathBuf)> = sources
        .iter()
        .map(|source| {
            let parsed = micro_extensions::Source::parse(source).map_err(anyhow::Error::msg)?;
            Ok((
                source.clone(),
                parsed.install_path(&home, &workspace, false),
            ))
        })
        .collect::<Result<_>>()?;
    let capabilities = capabilities_of(&home, &workspace, &installed).await;

    for (source, path) in installed {
        let state = match path.exists() {
            true => "installed",
            false => "missing",
        };
        println!("{source:<40} {state:<10} {}", path.display());
        if let Some(described) = capabilities.get(&source) {
            for line in described {
                println!("{:<40} {line}", "");
            }
        }
    }
    Ok(())
}

async fn capabilities_of(
    home: &Path,
    workspace: &Path,
    installed: &[(String, PathBuf)],
) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut entries: Vec<PathBuf> = Vec::new();
    let mut owners: Vec<(String, PathBuf)> = Vec::new();
    for (source, path) in installed {
        let found = match path.is_dir() {
            true => micro_extensions::entries_of(path)
                .unwrap_or_else(|| micro_extensions::in_directory(path)),
            false => vec![path.clone()],
        };
        for entry in found {
            owners.push((source.clone(), entry.clone()));
            entries.push(entry);
        }
    }
    if entries.is_empty() {
        return Default::default();
    }

    let loaded =
        match micro_extensions::Host::start(home, &entries, workspace, false, false, "print").await
        {
            Ok(host) => {
                let loaded = host.loaded().clone();
                host.shutdown("quit").await;
                loaded
            }
            Err(_) => return Default::default(),
        };

    let roots: Vec<(PathBuf, String)> = installed
        .iter()
        .filter_map(|(_, path)| {
            micro_extensions::package_name(path).map(|named| (path.clone(), named))
        })
        .collect();
    let resolved = crate::capabilities::resolve(&loaded, &roots, true, false).await;

    let mut described: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for grant in resolved.grants.all() {
        let Some((source, _)) = owners
            .iter()
            .find(|(_, entry)| entry.display().to_string() == grant.path)
        else {
            continue;
        };
        described.entry(source.clone()).or_default().push(format!(
            "{}  {}",
            grant.name,
            crate::capabilities::describe(grant)
        ));
    }
    described
}

/// Add a source to the settings, or take it out.
fn remember(source: &str, keep: bool) -> Result<bool> {
    let path = micro_config::default_path()?;
    let mut config = micro_config::Config::load_from(&path)?;
    let mut sources = config.extensions.clone().unwrap_or_default();

    let held = sources.iter().any(|held| held == source);
    match (keep, held) {
        (true, true) | (false, false) => return Ok(false),
        (true, false) => sources.push(source.to_string()),
        (false, true) => sources.retain(|held| held != source),
    }

    config.extensions = Some(sources);
    config.save_to(&path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::from_json(
            r#"{"providers": {
                "anthropic": {"base_url": "https://a.test", "api": "anthropic-messages",
                    "models": [{"id": "claude-sonnet-5"}]},
                "openrouter": {"base_url": "https://o.test", "api": "openai-completions",
                    "models": [{"id": "anthropic/claude-sonnet-5"}, {"id": "only-here"}]},
                "proxy-one": {"base_url": "https://p1.test", "api": "openai-completions",
                    "models": [{"id": "shared-model"}]},
                "proxy-two": {"base_url": "https://p2.test", "api": "openai-completions",
                    "models": [{"id": "shared-model"}]}
            }}"#,
        )
        .unwrap()
    }

    fn store(label: &str) -> AuthStore {
        let directory =
            std::env::temp_dir().join(format!("micro-cli-auth-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        AuthStore::open_at(directory.join("auth.json")).unwrap()
    }

    fn target(name: Option<&str>, provider: Option<&str>, model: Option<&str>) -> AuthTarget {
        AuthTarget {
            name: name.map(str::to_string),
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
        }
    }

    #[test]
    fn a_target_names_a_provider_or_a_model_that_has_one() {
        let catalog = catalog();
        let store = store("target");

        let resolve = |target: AuthTarget| resolve_target(&target, &catalog, &store);
        assert_eq!(
            resolve(target(Some("claude"), None, None)).unwrap(),
            "anthropic"
        );
        assert_eq!(
            resolve(target(Some("only-here"), None, None)).unwrap(),
            "openrouter"
        );
        assert_eq!(
            resolve(target(None, Some("openrouter"), Some("only-here"))).unwrap(),
            "openrouter"
        );
        assert!(resolve(target(None, Some("anthropic"), Some("only-here"))).is_err());
        assert!(resolve(target(None, Some("nowhere"), None))
            .unwrap_err()
            .to_string()
            .starts_with("unknown provider"));
        assert!(resolve(target(None, None, None)).is_err());
    }

    #[test]
    fn a_model_two_providers_serve_needs_the_one_that_is_configured() {
        let catalog = catalog();
        let store = store("ambiguous");

        let error = resolve_target(&target(None, None, Some("shared-model")), &catalog, &store)
            .unwrap_err();
        assert!(error.to_string().contains("--provider"), "{error}");

        store.store_api_key("proxy-two", "key").unwrap();
        assert_eq!(
            resolve_target(&target(None, None, Some("shared-model")), &catalog, &store).unwrap(),
            "proxy-two"
        );
    }

    #[test]
    fn durations_take_a_unit() {
        assert_eq!(parse_duration_ms("30m"), Some(1_800_000));
        assert_eq!(parse_duration_ms("1h"), Some(3_600_000));
        assert_eq!(parse_duration_ms("45s"), Some(45_000));
        assert_eq!(parse_duration_ms("250ms"), Some(250));
        assert_eq!(parse_duration_ms("30"), None);
        assert_eq!(parse_duration_ms("m"), None);
        assert_eq!(parse_duration_ms("3d"), None);
    }

    #[test]
    fn an_option_is_picked_by_number_or_id_and_defaults_to_the_first() {
        let options = vec![
            micro_auth::LoginOption {
                id: "browser".into(),
                label: "Browser login (default)".into(),
            },
            micro_auth::LoginOption {
                id: "copy_code".into(),
                label: "Copy code login (headless)".into(),
            },
        ];
        assert_eq!(pick("", &options), Some("browser"));
        assert_eq!(pick("2", &options), Some("copy_code"));
        assert_eq!(pick("COPY_CODE", &options), Some("copy_code"));
        assert_eq!(pick("3", &options), None);
        assert_eq!(pick("0", &options), None);
    }

    #[tokio::test]
    async fn a_check_reports_readiness_and_the_credential_when_asked() {
        let store = store("check");
        let options = CheckOptions {
            json: false,
            credentials: false,
            refresh: true,
        };

        let missing = check(&store, "a-proxy".into(), options).await;
        assert_eq!(missing.exit_code(), 1);
        assert_eq!(missing.reason, Some("credentials_not_configured"));

        store.store_api_key("a-proxy", "proxy-key").unwrap();
        let ready = check(&store, "a-proxy".into(), options).await;
        assert_eq!(ready.render(false), "ready");
        assert_eq!(ready.exit_code(), 0);
        assert_eq!(
            ready.render(true),
            r#"{"status":"ready","provider":"a-proxy","authType":"api_key"}"#
        );

        let with_credential = check(
            &store,
            "a-proxy".into(),
            CheckOptions {
                credentials: true,
                ..options
            },
        )
        .await;
        assert_eq!(with_credential.render(false), "proxy-key");
    }

    #[tokio::test]
    async fn a_check_without_refresh_reports_a_stored_token_as_it_stands() {
        let store = store("check-no-refresh");
        store
            .set(
                "anthropic",
                Credential::OAuth(micro_auth::OAuthCredential {
                    access_token: "sk-ant-oat01-stale".into(),
                    refresh_token: "r".into(),
                    expires: 1,
                    client_id: None,
                }),
            )
            .unwrap();

        let result = check(
            &store,
            "anthropic".into(),
            CheckOptions {
                json: true,
                credentials: true,
                refresh: false,
            },
        )
        .await;
        assert_eq!(result.auth_type, Some("oauth"));
        assert_eq!(result.credentials.as_deref(), Some("sk-ant-oat01-stale"));
    }
}
