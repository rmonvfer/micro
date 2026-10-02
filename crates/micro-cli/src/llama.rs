//! `micro llama`: point micro at a llama.cpp router, and list, search, download, load and unload
//! its models.

use anyhow::bail;
use anyhow::Result;
use micro_auth::AuthStore;
use micro_provider::llama_cpp;
use micro_provider::llama_cpp::huggingface;
use micro_provider::llama_cpp::LlamaClient;
use micro_provider::llama_cpp::Progress;
use std::io::Write as _;

/// The router micro is pointed at, or why there is none.
fn client() -> Result<LlamaClient> {
    let config_home = micro_dirs::config_dir().unwrap_or_default();
    let Some(url) = llama_cpp::configured_url(&config_home) else {
        bail!(
            "no llama.cpp router is configured: run `micro llama connect {}` or set {}",
            llama_cpp::DEFAULT_SERVER_URL,
            llama_cpp::BASE_URL_ENV
        );
    };
    let store = AuthStore::open()?;
    LlamaClient::new(&url, llama_cpp::api_key(&store)).map_err(anyhow::Error::msg)
}

/// Remember `url` as the router to use, after checking it answers as a router. A key, when one is
/// given, is stored as the `llama.cpp` credential.
pub async fn connect(url: Option<&str>, api_key: Option<&str>) -> Result<()> {
    let url = url.unwrap_or(llama_cpp::DEFAULT_SERVER_URL);
    let store = AuthStore::open()?;
    if let Some(key) = api_key.filter(|key| !key.trim().is_empty()) {
        store.set(
            llama_cpp::PROVIDER,
            micro_auth::Credential::api_key(key.trim()),
        )?;
    }
    let client = LlamaClient::new(url, llama_cpp::api_key(&store)).map_err(anyhow::Error::msg)?;
    let listed = client.list(false).await.map_err(anyhow::Error::msg)?;
    let config_home = micro_dirs::config_dir().unwrap_or_default();
    let url =
        llama_cpp::remember_url(&config_home, client.server_url()).map_err(anyhow::Error::msg)?;
    println!(
        "Connected to the llama.cpp router at {url}: {} models.",
        listed.len()
    );
    if !listed.iter().any(|model| model.is_loaded()) {
        println!(
            "No model is loaded yet: `micro llama load <model>` loads one for `/model` to select."
        );
    }
    Ok(())
}

/// The router's models and the state each is in.
pub async fn status() -> Result<()> {
    let client = client()?;
    let listed = client.list(false).await.map_err(anyhow::Error::msg)?;
    let data_home = micro_dirs::data_dir().unwrap_or_default();
    let _ = llama_cpp::remember_context_windows(&data_home, &listed);
    let remembered = llama_cpp::remembered_context_windows(&data_home);

    println!("llama.cpp router at {}", client.server_url());
    if listed.is_empty() {
        println!("No models: check --models-dir and restart the router.");
        return Ok(());
    }
    let width = listed
        .iter()
        .map(|model| model.id.chars().count())
        .max()
        .unwrap_or(0);
    for model in &listed {
        let mut state = model.status.value.clone();
        if let Some((done, total)) = model.download_progress() {
            state = format!(
                "{state} {} / {}",
                llama_cpp::format_bytes(done),
                llama_cpp::format_bytes(total)
            );
        }
        if model.status.failed {
            state.push_str(" (failed)");
        }
        println!(
            "{:<width$}  {:<12} {:>8} ctx",
            model.id,
            state,
            model.context_window(remembered.get(&model.id).copied())
        );
    }
    Ok(())
}

/// GGUF repositories on Hugging Face, or the quantizations of one when `query` names it.
pub async fn search(query: &str) -> Result<()> {
    let hugging_face =
        huggingface::HuggingFace::new(huggingface::DEFAULT_URL, huggingface::find_token());
    let query = query.trim();
    if query.contains('/') {
        let details = hugging_face
            .details(query)
            .await
            .map_err(anyhow::Error::msg)?;
        if let Some(how) = &details.gated {
            println!(
                "{} is gated ({how}): request access at https://huggingface.co/{} first.",
                details.id, details.id
            );
        }
        if details.quantizations.is_empty() {
            println!("{} has no GGUF quantizations.", details.id);
            return Ok(());
        }
        for quantization in &details.quantizations {
            let size = quantization
                .size
                .map(llama_cpp::format_bytes)
                .unwrap_or_else(|| "size unknown".to_string());
            println!("{}:{}  {size}", details.id, quantization.name);
        }
        return Ok(());
    }
    let found = hugging_face
        .search(query)
        .await
        .map_err(anyhow::Error::msg)?;
    if found.is_empty() {
        println!("No GGUF repositories match \"{query}\".");
    }
    for repository in found {
        println!(
            "{:<60} {:>10} downloads",
            repository.id, repository.downloads
        );
    }
    Ok(())
}

/// Have the router download `owner/repository[:quant]`, showing the bytes as they arrive.
pub async fn download(model: &str) -> Result<()> {
    let repository = model.split(':').next().unwrap_or(model);
    let hugging_face =
        huggingface::HuggingFace::new(huggingface::DEFAULT_URL, huggingface::find_token());
    if let Ok(details) = hugging_face.details(repository).await {
        if let Some(how) = details.gated {
            println!(
                "{repository} is gated ({how}). The router downloads it, so its process needs HF_TOKEN \
                 for an account with access: https://huggingface.co/{repository}"
            );
        }
    }
    let client = client()?;
    let listed = client
        .download_and_wait(model, show)
        .await
        .map_err(anyhow::Error::msg)?;
    finish_line();
    println!(
        "Downloaded {model}. The router now lists {} models.",
        listed.len()
    );
    Ok(())
}

/// Load `model`, unloading the others first when asked to. Nothing is unloaded unasked.
pub async fn load(model: &str, unload_others: bool) -> Result<()> {
    let client = client()?;
    let listed = client.list(false).await.map_err(anyhow::Error::msg)?;
    if !listed.iter().any(|candidate| candidate.id == model) {
        bail!("the router has no model {model}; `micro llama status` lists them");
    }
    let others: Vec<&str> = listed
        .iter()
        .filter(|candidate| candidate.id != model && candidate.is_loaded())
        .map(|candidate| candidate.id.as_str())
        .collect();
    if unload_others {
        for other in &others {
            println!("Unloading {other}");
            client
                .unload_and_wait(other)
                .await
                .map_err(anyhow::Error::msg)?;
        }
    } else if !others.is_empty() {
        println!(
            "Keeping {} loaded; pass --unload-others to unload them first.",
            others.join(", ")
        );
    }
    let loaded = client
        .load_and_wait(model, show)
        .await
        .map_err(anyhow::Error::msg)?;
    finish_line();
    let data_home = micro_dirs::data_dir().unwrap_or_default();
    let _ = llama_cpp::remember_context_windows(&data_home, std::slice::from_ref(&loaded));
    println!(
        "Loaded {model} with a {} token context window. Select it with `/model {}/{model}`.",
        loaded.context_window(None),
        llama_cpp::PROVIDER
    );
    Ok(())
}

pub async fn unload(model: &str) -> Result<()> {
    let client = client()?;
    client
        .unload_and_wait(model)
        .await
        .map_err(anyhow::Error::msg)?;
    println!("Unloaded {model}.");
    Ok(())
}

/// Rewrite the progress line in place.
fn show(progress: Progress) {
    let mut line = progress.message;
    if let Some(ratio) = progress.ratio {
        line.push_str(&format!(" {:>3.0}%", ratio * 100.0));
    }
    if let Some(detail) = progress.detail {
        line.push_str(&format!("  {detail}"));
    }
    print!("\r\x1b[2K{line}");
    let _ = std::io::stdout().flush();
}

fn finish_line() {
    println!();
}
