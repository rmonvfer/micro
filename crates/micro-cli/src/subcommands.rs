//! The non-conversational commands: credentials, the model catalog, and saved sessions.

use anyhow::bail;
use anyhow::Context as _;
use anyhow::Result;
use micro_auth::AuthStore;
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
    // Wide enough for the longest name on the list, so the columns line up whatever is
    // on it.
    let width = listed
        .iter()
        .map(|status| status.provider.chars().count())
        .max()
        .unwrap_or(0);

    for status in listed {
        // A credential can be stored and still be blank, which reads as "ready" everywhere
        // else and as a missing authentication header at the provider. Say so here.
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
            micro_auth::CredentialSource::Missing => String::new(),
        };
        println!("{:<width$}  {:<14} {source}", status.provider, state);
    }
    Ok(())
}

pub async fn auth_login(provider: &str) -> Result<()> {
    let store = AuthStore::open()?;
    match store.begin_login(provider).await? {
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
        }
        LoginFlow::DeviceCode(pending) => {
            println!("Open {}", pending.verification_uri());
            println!("Enter the code: {}", pending.user_code());
            println!("Waiting for authorization…");
            store.complete_device_login(&pending).await?;
            println!("Signed in to {}.", pending.provider);
        }
    }
    Ok(())
}

/// Adopt the credentials agent47 already holds, so a user with a working setup does not
/// have to authenticate a second time.
pub async fn auth_import(overwrite: bool) -> Result<()> {
    let store = AuthStore::open()?;
    let report = store.import_agent47(overwrite)?;

    println!("Read {}", report.source.display());
    for entry in &report.entries {
        println!("  {:<16} {}", entry.provider, entry.outcome.reason());
    }

    match report.imported() {
        0 => println!("\nNothing imported."),
        count => println!("\nImported {count} credential(s). Run `micro auth status` to check."),
    }
    Ok(())
}

pub async fn auth_logout(provider: &str) -> Result<()> {
    AuthStore::open()?.remove(provider)?;
    println!("Removed the stored credential for {provider}.");
    Ok(())
}

pub async fn models(query: Option<&str>, live: bool) -> Result<()> {
    let mut catalog = Catalog::load().unwrap_or_else(|_| Catalog::bundled());
    if live {
        let store = AuthStore::open()?;
        let client = reqwest::Client::new();
        let copilot = store.resolve(micro_auth::GITHUB_COPILOT).await.ok();
        // The token says which host serves this account; only an individual plan is
        // served by the default one.
        let copilot_base = copilot
            .as_ref()
            .and_then(|credential| micro_auth::copilot::base_url_from_token(credential.token()));
        let credentials = copilot
            .as_ref()
            .map(|credential| micro_models::CopilotCredentials {
                token: credential.token(),
                base_url: copilot_base
                    .as_deref()
                    .unwrap_or(micro_models::COPILOT_BASE_URL),
            });
        for failure in catalog.merge_live_listings(&client, credentials).await {
            eprintln!("note: {failure}");
        }
    }

    let models = match query {
        Some(query) => catalog.resolve(query).candidates(),
        None => catalog.models().iter().collect(),
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

/// `micro sessions show <id>` — what the session's ledger recorded.
///
/// Without a turn, the turns it holds; with one, what the model was shown at that turn,
/// each stretch of the prompt named by whoever supplied it. With `--raw`, the request
/// itself, rebuilt from what was recorded and printed as it went out.
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
        println!(
            "No turns recorded. A session written before the ledger existed holds its \
             conversation and nothing else."
        );
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

/// `micro sessions export <id>` — the whole ledger, as it is on disk.
///
/// Every line, in the order it was written: the conversation, the facts recorded beside
/// it, and the records that say where the conversation is read from. A line that cannot
/// be read is counted rather than printed, so what comes out is JSONL throughout.
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

/// `micro bill [session] [--diff <turn>]` — what a session cost, and what it went on.
///
/// The reading of the ledger is [`micro_commands::bill`], the same one `/bill` shows.
/// Priced against the catalog as it stands, which is what makes an old session billable at
/// all: the ledger recorded what was used, not what it was worth at the time.
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

/// `micro why-miss <session> [turn]` — why a turn paid for a prompt the provider had.
///
/// The reading of the ledger is [`micro_commands::why_miss`], the same one `/why-miss`
/// shows, so what a session says about itself does not depend on where it was asked.
pub async fn why_miss(id: &str, turn: Option<u64>) -> Result<()> {
    let store = SessionStore::from_env()?;
    let explanation = micro_commands::why_miss(&store, id, turn)
        .await
        .map_err(|reason| anyhow::anyhow!(reason))?;
    println!("{explanation}");
    Ok(())
}

/// Every turn the session recorded a request for, in order and without repeats.
///
/// A turn re-issued after a transient failure was recorded once per attempt, and the last
/// attempt is the one that produced the answer, so it is the one kept here.
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
        // Named by the entry it was read from where there is one. The summary a
        // compaction left in its place is not an entry and has no name.
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

/// The request as it went out, rebuilt from what the turn recorded.
fn print_request(id: &str, turn: &micro_session::ReconstructedTurn) -> Result<()> {
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
        // The session names the conversation, which is what a provider that caches a
        // prompt is told; it was the session's own id then and it is now.
        cache_key: Some(id.to_string()),
    };
    let payload = micro_provider::client_for_model(model).payload(&turn.model, &context);
    let body = serde_json::to_vec(&payload)?;

    // The record says what was sent. If what was rebuilt hashes to something else, the
    // rebuilding is what is wrong, and saying so is better than printing it as if it were
    // the request.
    if micro_types::content_hash(&body) != turn.request_hash {
        eprintln!(
            "note: this rebuilds to a different request than the one recorded ({}). \
             Something changed the request after it was recorded.",
            short(&turn.request_hash)
        );
    }
    println!("{}", serde_json::to_string_pretty(&payload)?);
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

/// `micro install <source>` — fetch a package and remember it.
///
/// The source is written into the settings, so the next run loads it without being told
/// again. A package that will not fetch leaves the settings alone.
pub async fn install(source: &str, local: bool, workspace: &Path) -> Result<()> {
    let parsed = micro_extensions::Source::parse(source).map_err(anyhow::Error::msg)?;
    let home = micro_context::micro_home()?;

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

    // What it registered is worth seeing now rather than at the next start. A package is a
    // directory rather than a file, and what it loads is whatever its own manifest names —
    // the same reading the next start will do, rather than handing the directory itself to
    // a loader that only takes files.
    let workspace = std::env::current_dir().unwrap_or_default();
    let entries = match installed.path.is_dir() {
        true => micro_extensions::entries_of(&installed.path)
            .unwrap_or_else(|| micro_extensions::in_directory(&installed.path)),
        false => vec![installed.path.clone()],
    };
    match micro_extensions::Host::start(
        &home,
        &entries,
        &workspace,
        false,
        false,
        "print",
    )
    .await
    {
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

/// `micro remove <source>` — take a package away and forget it.
///
/// Its own `deactivate` runs first, while its files are still there to run: an extension
/// that started something, wrote something, or registered something outside micro is given
/// the chance to put it back, which is the only chance it will get.
pub async fn remove(source: &str, local: bool, workspace: &Path) -> Result<()> {
    let parsed = micro_extensions::Source::parse(source).map_err(anyhow::Error::msg)?;
    let home = micro_context::micro_home()?;

    deactivate(&parsed.install_path(&home, workspace, local), &home, workspace).await;
    micro_extensions::remove(&parsed, &home, workspace, local).map_err(anyhow::Error::msg)?;
    let forgotten = remember(&parsed.canonical(), false)?;

    match forgotten {
        true => println!("Removed {}.", parsed.canonical()),
        false => println!("{} was not installed.", parsed.canonical()),
    }
    Ok(())
}

/// Let a package's extensions go before its files do.
///
/// Nothing here is fatal, and nothing here is required to succeed: an extension that will
/// not load cannot be deactivated either, and one that throws on the way out is still being
/// removed. What this buys is the one moment an extension has to undo what it did outside
/// micro, which no amount of deleting files afterwards can give it.
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
    let Ok(host) = micro_extensions::Host::start(home, &entries, workspace, false, false, "print")
        .await
    else {
        return;
    };
    for extension in &host.loaded().extensions {
        if let Err(error) = host.deactivate(&extension.path).await {
            println!("  note     {} did not deactivate cleanly: {error}", extension.path);
        }
    }
    host.shutdown("quit").await;
}

/// `micro list` — what is installed, and whether it is still there.
pub async fn list_packages() -> Result<()> {
    let path = micro_config::default_path()?;
    let config = micro_config::Config::load_from(&path)?;
    let sources = config.extensions.clone().unwrap_or_default();

    if sources.is_empty() {
        println!("No extension packages installed.");
        return Ok(());
    }

    let home = micro_context::micro_home()?;
    let workspace = std::env::current_dir().unwrap_or_default();

    // What each package's own extensions may do. Worked out for the whole listing at once,
    // in one host: a capability set an extension declared is read from its manifest, but a
    // legacy one's has to be derived from what it registers, and that means loading it.
    let installed: Vec<(String, PathBuf)> = sources
        .iter()
        .map(|source| {
            let parsed = micro_extensions::Source::parse(source).map_err(anyhow::Error::msg)?;
            Ok((source.clone(), parsed.install_path(&home, &workspace, false)))
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

/// What each installed package's extensions may do, by the source they were installed from.
///
/// One host for the whole listing rather than one per package: a legacy extension's set is
/// derived from what it registers, so it has to be loaded to be described, and loading them
/// separately would pay for a Bun process apiece. Without a runtime to load them there is
/// nothing to derive from, and the listing simply says less rather than failing.
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

    let loaded = match micro_extensions::Host::start(home, &entries, workspace, false, false, "print")
        .await
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
        described
            .entry(source.clone())
            .or_default()
            .push(format!("{}  {}", grant.name, crate::capabilities::describe(grant)));
    }
    described
}

/// Add a source to the settings, or take it out. Says whether anything changed.
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
