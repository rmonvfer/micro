//! The non-conversational commands: credentials, the model catalog, and saved sessions.

use anyhow::bail;
use anyhow::Context as _;
use anyhow::Result;
use micro_auth::AuthStore;
use micro_auth::LoginFlow;
use micro_models::Catalog;
use micro_session::SessionStore;
use std::io::BufRead as _;
use std::path::Path;
use std::io::Write as _;

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
        let credentials = copilot
            .as_ref()
            .map(|credential| micro_models::CopilotCredentials {
                token: credential.token(),
                base_url: micro_models::COPILOT_BASE_URL,
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
    let home = micro_policy::micro_home()?;

    println!("Installing {}...", parsed.canonical());
    let installed = micro_extensions::install(&parsed, &home, workspace, local)
        .await
        .map_err(anyhow::Error::msg)?;

    remember(&installed.source, true)?;
    println!("Installed {} to {}", installed.source, installed.path.display());

    // What it registered is worth seeing now rather than at the next start.
    match micro_extensions::Host::start(&home, std::slice::from_ref(&installed.path)).await {
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
                println!("  warning  {} did not load: {}", failure.path, failure.error);
            }
            host.shutdown().await;
        }
        Err(error) => println!("  note     {error}"),
    }
    Ok(())
}

/// `micro remove <source>` — take a package away and forget it.
pub async fn remove(source: &str, local: bool, workspace: &Path) -> Result<()> {
    let parsed = micro_extensions::Source::parse(source).map_err(anyhow::Error::msg)?;
    let home = micro_policy::micro_home()?;

    micro_extensions::remove(&parsed, &home, workspace, local).map_err(anyhow::Error::msg)?;
    let forgotten = remember(&parsed.canonical(), false)?;

    match forgotten {
        true => println!("Removed {}.", parsed.canonical()),
        false => println!("{} was not installed.", parsed.canonical()),
    }
    Ok(())
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

    let home = micro_policy::micro_home()?;
    let workspace = std::env::current_dir().unwrap_or_default();
    for source in sources {
        let parsed = micro_extensions::Source::parse(&source).map_err(anyhow::Error::msg)?;
        let path = parsed.install_path(&home, &workspace, false);
        let state = match path.exists() {
            true => "installed",
            false => "missing",
        };
        println!("{source:<40} {state:<10} {}", path.display());
    }
    Ok(())
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
