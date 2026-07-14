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

pub async fn auth_status() -> Result<()> {
    let store = AuthStore::open()?;
    for status in store.status() {
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
        println!("{:<16} {:<14} {}", status.provider, state, source);
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
