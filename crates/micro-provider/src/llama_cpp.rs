//! The llama.cpp router: a `llama-server` started without a model, which discovers GGUF files,
//! loads and unloads them on request, and downloads more from Hugging Face.
//!
//! Its loaded models are chat models under the `llama.cpp` provider, spoken to over the
//! OpenAI-compatible `/v1`, and each is also a classifier model with the same id.

use micro_auth::AuthStore;
use micro_models::Modality;
use micro_models::ModelCost;
use micro_models::ModelDef;
use micro_models::WireApi;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

/// The provider id llama.cpp models are listed under.
pub const PROVIDER: &str = "llama.cpp";

/// Where a router is looked for when nothing says otherwise.
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8080";

/// The environment variables that point micro at a router and authenticate it.
pub const BASE_URL_ENV: &str = "LLAMA_BASE_URL";
pub const API_KEY_ENV: &str = "LLAMA_API_KEY";

/// The file, in micro's configuration directory, that remembers which router to use.
pub const CONNECTION_FILE: &str = "llama-cpp.json";

/// The file, in micro's data directory, that remembers each model's context window between runs.
pub const CONTEXT_CACHE_FILE: &str = "llama-cpp-context.json";

/// The context window assumed of a model nothing says anything about.
const FALLBACK_CONTEXT_WINDOW: u32 = 128_000;

/// How long one request to the router may take before it is given up on.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// How often a load or a download is checked on.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Whether `provider` is served without any credential.
pub fn is_keyless(provider: &str) -> bool {
    provider == PROVIDER
}

/// The key a router was started with, when it was started with one.
pub fn api_key(store: &AuthStore) -> Option<String> {
    let stored = store
        .get(PROVIDER)
        .map(|credential| credential.token().to_string());
    stored
        .or_else(|| std::env::var(API_KEY_ENV).ok())
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

/// Which router to use, as remembered by `micro llama connect`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Connection {
    pub url: String,
}

/// The router micro is pointed at: `LLAMA_BASE_URL` first, then the remembered connection.
pub fn configured_url(config_dir: &Path) -> Option<String> {
    if let Some(url) = std::env::var(BASE_URL_ENV)
        .ok()
        .filter(|url| !url.trim().is_empty())
    {
        return normalize_server_url(&url).ok();
    }
    let text = std::fs::read_to_string(config_dir.join(CONNECTION_FILE)).ok()?;
    let connection: Connection = serde_json::from_str(&text).ok()?;
    normalize_server_url(&connection.url).ok()
}

/// Remember `url` as the router to use.
pub fn remember_url(config_dir: &Path, url: &str) -> Result<String, String> {
    let url = normalize_server_url(url)?;
    std::fs::create_dir_all(config_dir).map_err(|error| error.to_string())?;
    let text = serde_json::to_string_pretty(&Connection { url: url.clone() })
        .map_err(|error| error.to_string())?;
    std::fs::write(config_dir.join(CONNECTION_FILE), format!("{text}\n"))
        .map_err(|error| error.to_string())?;
    Ok(url)
}

/// The router's root address: no trailing slash, no `/v1`, no query.
pub fn normalize_server_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    let Some((scheme, rest)) = value.split_once("://") else {
        return Err(format!("{value} is not a server URL"));
    };
    if scheme != "http" && scheme != "https" {
        return Err("Server URL must use http or https".to_string());
    }
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let rest = rest.trim_end_matches('/');
    let rest = rest
        .strip_suffix("/v1")
        .unwrap_or(rest)
        .trim_end_matches('/');
    if rest.is_empty() {
        return Err(format!("{value} names no host"));
    }
    Ok(format!("{scheme}://{rest}"))
}

/// The OpenAI-compatible address chat requests go to.
pub fn inference_url(server_url: &str) -> String {
    format!("{server_url}/v1")
}

/// What the router says about one model.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RouterModel {
    pub id: String,
    pub status: RouterStatus,
    #[serde(default)]
    pub architecture: Option<RouterArchitecture>,
    /// Where the router found it: a file under `--models-dir`, a preset, or the cache.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub meta: Option<RouterMeta>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RouterStatus {
    /// `unloaded`, `loading`, `loaded`, `downloading` or `sleeping`.
    pub value: String,
    /// The arguments the model's server was started with.
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub failed: bool,
    #[serde(default)]
    pub exit_code: Option<i64>,
    /// Bytes done and expected per file, while downloading.
    #[serde(default)]
    pub progress: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct RouterArchitecture {
    #[serde(default)]
    pub input_modalities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct RouterMeta {
    /// The context window the model is running with.
    #[serde(default)]
    pub n_ctx: Option<u32>,
    /// The context window the model was trained for.
    #[serde(default)]
    pub n_ctx_train: Option<u32>,
    #[serde(default)]
    pub size: Option<u64>,
}

impl RouterModel {
    pub fn is_loaded(&self) -> bool {
        self.status.value == "loaded"
    }

    /// Whether a request may name it: loaded, asleep (a request wakes it), or a preset the router
    /// will load on first use.
    pub fn is_selectable(&self, autoload: bool) -> bool {
        match self.status.value.as_str() {
            "loaded" | "sleeping" => true,
            "unloaded" => {
                autoload && !self.status.failed && self.source.as_deref() == Some("preset")
            }
            _ => false,
        }
    }

    /// The `-c`/`--ctx-size` it was started with.
    fn configured_context_window(&self) -> Option<u32> {
        self.status
            .args
            .windows(2)
            .find(|pair| matches!(pair[0].as_str(), "--ctx-size" | "-c" | "-ctx"))
            .and_then(|pair| pair[1].parse::<u32>().ok())
            .filter(|window| *window > 0)
    }

    /// What the model holds: what it runs with, else what it was started with, else what an
    /// earlier run saw it run with, else what it was trained for.
    pub fn context_window(&self, cached: Option<u32>) -> u32 {
        let meta = self.meta.clone().unwrap_or_default();
        meta.n_ctx
            .filter(|window| *window > 0)
            .or_else(|| self.configured_context_window())
            .or(cached.filter(|window| *window > 0))
            .or(meta.n_ctx_train.filter(|window| *window > 0))
            .unwrap_or(FALLBACK_CONTEXT_WINDOW)
    }

    /// The window the model is actually running with, which is what is worth remembering.
    pub fn runtime_context_window(&self) -> Option<u32> {
        if !self.is_loaded() {
            return None;
        }
        self.meta
            .as_ref()
            .and_then(|meta| meta.n_ctx)
            .filter(|window| *window > 0)
            .or_else(|| self.configured_context_window())
    }

    /// Bytes downloaded so far and in all, across every file, when the router says.
    pub fn download_progress(&self) -> Option<(u64, u64)> {
        progress_totals(self.status.progress.as_ref()?)
    }
}

/// What the router says about itself.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct RouterProps {
    #[serde(default)]
    pub models_autoload: Option<bool>,
    #[serde(default)]
    pub chat_template: Option<String>,
}

/// How far a load or a download has got, for whoever is showing it.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub message: String,
    /// From 0 to 1, when the router says.
    pub ratio: Option<f64>,
    pub detail: Option<String>,
}

fn progress_totals(progress: &Value) -> Option<(u64, u64)> {
    let files = progress.get("progress").unwrap_or(progress).as_object()?;
    let (mut done, mut total) = (0u64, 0u64);
    for entry in files.values() {
        let (Some(file_done), Some(file_total)) = (
            entry.get("done").and_then(Value::as_u64),
            entry.get("total").and_then(Value::as_u64),
        ) else {
            continue;
        };
        done += file_done;
        total += file_total;
    }
    (total > 0).then_some((done, total))
}

/// A byte count as a person reads it.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let units = ["KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = units[0];
    for next in &units[1..] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    match value >= 10.0 {
        true => format!("{value:.1} {unit}"),
        false => format!("{value:.2} {unit}"),
    }
}

/// A connection to one router.
#[derive(Clone)]
pub struct LlamaClient {
    server_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl LlamaClient {
    pub fn new(server_url: &str, api_key: Option<String>) -> Result<LlamaClient, String> {
        Ok(LlamaClient {
            server_url: normalize_server_url(server_url)?,
            api_key: api_key.filter(|key| !key.trim().is_empty()),
            http: reqwest::Client::new(),
        })
    }

    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.server_url))
            .timeout(REQUEST_TIMEOUT);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("cannot reach llama.cpp at {}: {error}", self.server_url))?;
        let status = response.status();
        let payload: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let message = payload
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("llama.cpp returned HTTP {}", status.as_u16()));
            return Err(message);
        }
        Ok(payload)
    }

    /// Every model the router knows, and what state each is in. `reload` has it look at its model
    /// directory again first.
    pub async fn list(&self, reload: bool) -> Result<Vec<RouterModel>, String> {
        let path = match reload {
            true => "/models?reload=1",
            false => "/models",
        };
        let payload = self.request(reqwest::Method::GET, path, None).await?;
        let data = payload
            .get("data")
            .and_then(Value::as_array)
            .ok_or("llama.cpp returned an invalid model catalog")?;
        data.iter()
            .map(|entry| {
                serde_json::from_value::<RouterModel>(entry.clone())
                    .map_err(|_| "Server is not running in llama.cpp router mode".to_string())
            })
            .collect()
    }

    /// What the router, or one model's server, says about itself, without loading anything.
    pub async fn props(&self, model: Option<&str>) -> Result<RouterProps, String> {
        let path = match model {
            Some(model) => format!("/props?model={}&autoload=false", encode(model)),
            None => "/props".to_string(),
        };
        let payload = self.request(reqwest::Method::GET, &path, None).await?;
        Ok(serde_json::from_value(payload).unwrap_or_default())
    }

    pub async fn load(&self, model: &str) -> Result<(), String> {
        self.request(
            reqwest::Method::POST,
            "/models/load",
            Some(json!({ "model": model })),
        )
        .await
        .map(|_| ())
    }

    pub async fn unload(&self, model: &str) -> Result<(), String> {
        self.request(
            reqwest::Method::POST,
            "/models/unload",
            Some(json!({ "model": model })),
        )
        .await
        .map(|_| ())
    }

    /// Have the router fetch `owner/repository[:quant]` from Hugging Face.
    pub async fn download(&self, model: &str) -> Result<(), String> {
        self.request(
            reqwest::Method::POST,
            "/models",
            Some(json!({ "model": model })),
        )
        .await
        .map(|_| ())
    }

    /// Unload `model` and wait until the router says it is.
    pub async fn unload_and_wait(&self, model: &str) -> Result<(), String> {
        self.unload(model).await?;
        loop {
            let listed = self.list(false).await?;
            match listed.iter().find(|entry| entry.id == model) {
                Some(entry) if entry.status.value != "unloaded" => {
                    tokio::time::sleep(POLL_INTERVAL).await
                }
                _ => return Ok(()),
            }
        }
    }

    /// Load `model` and wait until it is serving, reporting each state it passes through.
    pub async fn load_and_wait(
        &self,
        model: &str,
        mut on_progress: impl FnMut(Progress),
    ) -> Result<RouterModel, String> {
        self.load(model).await?;
        on_progress(Progress {
            message: "Loading model".to_string(),
            ratio: None,
            detail: None,
        });
        loop {
            let listed = self.list(false).await?;
            let Some(entry) = listed.into_iter().find(|entry| entry.id == model) else {
                return Err(format!("the router no longer lists {model}"));
            };
            if entry.is_loaded() {
                return Ok(entry);
            }
            if entry.status.failed {
                return Err(match entry.status.exit_code {
                    Some(code) => format!("Model exited with code {code}"),
                    None => "Model failed to load".to_string(),
                });
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    /// Download `model` and wait until the router lists it, reporting the bytes as they arrive.
    pub async fn download_and_wait(
        &self,
        model: &str,
        mut on_progress: impl FnMut(Progress),
    ) -> Result<Vec<RouterModel>, String> {
        self.download(model).await?;
        on_progress(Progress {
            message: "Downloading model".to_string(),
            ratio: None,
            detail: None,
        });
        let mut seen_downloading = false;
        let mut polls = 0;
        loop {
            let listed = self.list(false).await?;
            polls += 1;
            let entry = listed.iter().find(|entry| entry.id == model);
            match entry {
                Some(entry) if entry.status.value == "downloading" => {
                    seen_downloading = true;
                    if let Some((done, total)) = entry.download_progress() {
                        on_progress(Progress {
                            message: "Downloading model".to_string(),
                            ratio: Some(done as f64 / total as f64),
                            detail: Some(format!(
                                "{} / {}",
                                format_bytes(done),
                                format_bytes(total)
                            )),
                        });
                    }
                }
                Some(entry) if entry.status.failed => {
                    return Err(format!("the router could not download {model}"));
                }
                Some(_) if seen_downloading || polls >= 2 => return self.list(true).await,
                _ => {}
            }
            tokio::time::sleep(POLL_INTERVAL * 2).await;
        }
    }

    /// The router's models as catalog entries: a chat model and a classifier model for each one a
    /// request may name, with the context windows remembered from earlier runs filling in what an
    /// unloaded model cannot say.
    pub async fn catalog_models(
        &self,
        remembered: &BTreeMap<String, u32>,
    ) -> Result<Vec<ModelDef>, String> {
        let listed = self.list(false).await?;
        let autoload = match listed.iter().any(|model| {
            model.status.value == "unloaded" && model.source.as_deref() == Some("preset")
        }) {
            true => self
                .props(None)
                .await
                .ok()
                .and_then(|props| props.models_autoload)
                .unwrap_or(false),
            false => false,
        };

        let mut models = Vec::new();
        for model in listed.iter().filter(|model| model.is_selectable(autoload)) {
            // Only a loaded model gives its template without being woken or loaded for it.
            let props = match model.is_loaded() {
                true => self.props(Some(&model.id)).await.ok(),
                false => None,
            };
            let cached = remembered.get(&model.id).copied();
            models.push(chat_model(model, &self.server_url, props.as_ref(), cached));
            models.push(classifier_model(model, &self.server_url, cached));
        }
        Ok(models)
    }
}

/// One router model as a chat model.
pub fn chat_model(
    model: &RouterModel,
    server_url: &str,
    props: Option<&RouterProps>,
    cached_context_window: Option<u32>,
) -> ModelDef {
    let context_window = model.context_window(cached_context_window);
    let reasoning = props
        .and_then(|props| props.chat_template.as_deref())
        .is_some_and(|template| template.contains("enable_thinking"));
    let images = model
        .architecture
        .as_ref()
        .is_some_and(|architecture| architecture.input_modalities.iter().any(|m| m == "image"));
    let mut thinking = BTreeMap::new();
    if reasoning {
        for (level, value) in [
            ("off", Some("off")),
            ("minimal", None),
            ("low", None),
            ("medium", Some("medium")),
            ("high", None),
            ("xhigh", None),
        ] {
            thinking.insert(level.to_string(), value.map(str::to_string));
        }
    }
    let mut compat: micro_models::CompatOverrides = serde_json::from_value(json!({
        "supportsStore": false,
        "supportsDeveloperRole": false,
        "supportsReasoningEffort": false,
        "supportsUsageInStreaming": true,
        "supportsStrictMode": false,
        "maxTokensField": "max_tokens",
    }))
    .unwrap_or_default();
    if reasoning {
        if let Ok(with_thinking) = serde_json::from_value(json!({
            "supportsStore": false,
            "supportsDeveloperRole": false,
            "supportsReasoningEffort": false,
            "supportsUsageInStreaming": true,
            "supportsStrictMode": false,
            "maxTokensField": "max_tokens",
            "thinkingFormat": "qwen-chat-template",
        })) {
            compat = with_thinking;
        }
    }
    ModelDef {
        id: model.id.clone(),
        name: model.id.clone(),
        provider: PROVIDER.to_string(),
        api: WireApi::OpenaiCompletions,
        base_url: inference_url(server_url),
        context_window,
        max_output_tokens: context_window,
        reasoning,
        input: match images {
            true => vec![Modality::Text, Modality::Image],
            false => vec![Modality::Text],
        },
        output: Vec::new(),
        headers: BTreeMap::new(),
        aliases: Vec::new(),
        cost: ModelCost::default(),
        compat,
        thinking,
    }
}

/// The same router model as a classifier, read from next-token label probabilities.
pub fn classifier_model(
    model: &RouterModel,
    server_url: &str,
    cached_context_window: Option<u32>,
) -> ModelDef {
    let context_window = model.context_window(cached_context_window);
    ModelDef {
        id: model.id.clone(),
        name: model.id.clone(),
        provider: PROVIDER.to_string(),
        api: WireApi::LlamaCppClassify,
        base_url: server_url.to_string(),
        context_window,
        max_output_tokens: 1,
        reasoning: false,
        input: vec![Modality::Text],
        output: Vec::new(),
        headers: BTreeMap::new(),
        aliases: Vec::new(),
        cost: ModelCost::default(),
        compat: Default::default(),
        thinking: BTreeMap::new(),
    }
}

/// The context windows remembered from earlier runs, by model id.
pub fn remembered_context_windows(data_dir: &Path) -> BTreeMap<String, u32> {
    std::fs::read_to_string(context_cache_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Remember the windows loaded models are running with, so the same model unloaded later is not
/// described by its training context instead.
pub fn remember_context_windows(data_dir: &Path, models: &[RouterModel]) -> std::io::Result<()> {
    let mut remembered = remembered_context_windows(data_dir);
    let mut changed = false;
    for model in models {
        if let Some(window) = model.runtime_context_window() {
            changed |= remembered.insert(model.id.clone(), window) != Some(window);
        }
    }
    if !changed {
        return Ok(());
    }
    std::fs::create_dir_all(data_dir)?;
    let text = serde_json::to_string_pretty(&remembered).unwrap_or_default();
    std::fs::write(context_cache_path(data_dir), format!("{text}\n"))
}

fn context_cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CONTEXT_CACHE_FILE)
}

/// Percent-encode one query value.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Hugging Face, where the router downloads GGUF models from.
pub mod huggingface {
    use super::*;

    pub const DEFAULT_URL: &str = "https://huggingface.co";

    /// A repository a search found.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Found {
        pub id: String,
        pub downloads: u64,
    }

    /// One quantization a repository offers, and its size when every file of it says.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Quantization {
        pub name: String,
        pub size: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct Details {
        pub id: String,
        /// `None` when anyone may download it, otherwise how access is granted.
        pub gated: Option<String>,
        /// The usual choice, `Q4_K_M`, first, then smallest first.
        pub quantizations: Vec<Quantization>,
    }

    /// The token to search with: `HF_TOKEN`, then the files the Hugging Face tools write.
    pub fn find_token() -> Option<String> {
        if let Some(token) = std::env::var("HF_TOKEN")
            .ok()
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty())
        {
            return Some(token);
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let candidates = [
            std::env::var_os("HF_TOKEN_PATH").map(PathBuf::from),
            std::env::var_os("HF_HOME").map(|home| PathBuf::from(home).join("token")),
            std::env::var_os("XDG_CACHE_HOME")
                .map(|cache| PathBuf::from(cache).join("huggingface").join("token")),
            home.map(|home| home.join(".cache").join("huggingface").join("token")),
        ];
        candidates.into_iter().flatten().find_map(|path| {
            std::fs::read_to_string(path)
                .ok()
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty())
        })
    }

    pub struct HuggingFace {
        base_url: String,
        token: Option<String>,
        http: reqwest::Client,
    }

    impl HuggingFace {
        pub fn new(base_url: &str, token: Option<String>) -> HuggingFace {
            HuggingFace {
                base_url: base_url.trim_end_matches('/').to_string(),
                token,
                http: reqwest::Client::new(),
            }
        }

        async fn get(&self, path: &str) -> Result<Value, String> {
            let mut request = self
                .http
                .get(format!("{}{path}", self.base_url))
                .timeout(REQUEST_TIMEOUT);
            if let Some(token) = &self.token {
                request = request.bearer_auth(token);
            }
            let response = request
                .send()
                .await
                .map_err(|error| format!("cannot reach Hugging Face: {error}"))?;
            let status = response.status();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let payload: Value = response.json().await.unwrap_or(Value::Null);
            if status.as_u16() == 429 {
                return Err(match retry_after {
                    Some(seconds) => {
                        format!("Hugging Face rate limit reached; retry in {seconds}s")
                    }
                    None => "Hugging Face rate limit reached".to_string(),
                });
            }
            if !status.is_success() {
                return Err(payload
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Hugging Face returned HTTP {}", status.as_u16())));
            }
            Ok(payload)
        }

        /// GGUF repositories matching `query`, most downloaded first.
        pub async fn search(&self, query: &str) -> Result<Vec<Found>, String> {
            let payload = self
                .get(&format!(
                    "/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit=20",
                    encode(query)
                ))
                .await?;
            let listed = payload
                .as_array()
                .ok_or("Hugging Face returned invalid search results")?;
            Ok(listed
                .iter()
                .filter_map(|model| {
                    Some(Found {
                        id: model.get("id")?.as_str()?.to_string(),
                        downloads: model.get("downloads").and_then(Value::as_u64).unwrap_or(0),
                    })
                })
                .collect())
        }

        /// The quantizations a repository offers, and whether it is gated.
        pub async fn details(&self, id: &str) -> Result<Details, String> {
            let path: Vec<String> = id.split('/').map(encode).collect();
            let payload = self
                .get(&format!("/api/models/{}?blobs=true", path.join("/")))
                .await?;
            let mut sizes: BTreeMap<String, (u64, bool)> = BTreeMap::new();
            for file in payload
                .get("siblings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(name) = file.get("rfilename").and_then(Value::as_str) else {
                    continue;
                };
                let Some(quantization) = quantization_of(name) else {
                    continue;
                };
                let entry = sizes.entry(quantization).or_insert((0, true));
                match file.get("size").and_then(Value::as_u64) {
                    Some(size) => entry.0 += size,
                    None => entry.1 = false,
                }
            }
            let mut quantizations: Vec<Quantization> = sizes
                .into_iter()
                .map(|(name, (size, complete))| Quantization {
                    name,
                    size: complete.then_some(size),
                })
                .collect();
            quantizations.sort_by(|left, right| {
                (left.name != "Q4_K_M")
                    .cmp(&(right.name != "Q4_K_M"))
                    .then(
                        left.size
                            .unwrap_or(u64::MAX)
                            .cmp(&right.size.unwrap_or(u64::MAX)),
                    )
                    .then(left.name.cmp(&right.name))
            });
            let gated = match payload.get("gated") {
                Some(Value::String(how)) => Some(how.clone()),
                _ => None,
            };
            Ok(Details {
                id: payload
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .to_string(),
                gated,
                quantizations,
            })
        }
    }

    /// The quantization a GGUF file is, read from its name; projector files and other files are
    /// none.
    pub fn quantization_of(path: &str) -> Option<String> {
        let file = path.rsplit('/').next()?;
        let lower = file.to_ascii_lowercase();
        if !lower.ends_with(".gguf") || lower.starts_with("mmproj") {
            return None;
        }
        let mut stem = &file[..file.len() - 5];
        // `-00001-of-00003` marks one shard of a split file.
        if let Some(at) = stem.rfind("-of-") {
            let (head, tail) = stem.split_at(at);
            let shard = head.rsplit('-').next().unwrap_or_default();
            if tail.len() == 9
                && tail[4..].bytes().all(|byte| byte.is_ascii_digit())
                && shard.len() == 5
                && shard.bytes().all(|byte| byte.is_ascii_digit())
            {
                stem = &head[..head.len() - 6];
            }
        }
        let tail = stem.rsplit(['-', '_', '.']).collect::<Vec<_>>();
        // The quantization is the stem's last separated parts, read back to front until a part
        // starts one: `Q4_K_M` spans three parts, `UD-Q4_K_XL` four.
        for take in 1..=tail.len().min(5) {
            let candidate = tail[..take]
                .iter()
                .rev()
                .copied()
                .collect::<Vec<_>>()
                .join("_");
            let upper = candidate.to_ascii_uppercase();
            if is_quantization(&upper) {
                let with_ud = tail
                    .get(take)
                    .is_some_and(|part| part.eq_ignore_ascii_case("ud"));
                return Some(match with_ud {
                    true => format!("UD-{upper}"),
                    false => upper,
                });
            }
        }
        None
    }

    /// `Q4_K_M`, `IQ2_XXS`, `BF16`, `F16`, `F32`, `MXFP4` and their kind.
    fn is_quantization(name: &str) -> bool {
        let mut parts = name.split('_');
        let Some(head) = parts.next() else {
            return false;
        };
        let rest: Vec<&str> = parts.collect();
        let alnum =
            |part: &&str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric());
        let numbered = |prefix: &str| {
            head.strip_prefix(prefix).is_some_and(|digits| {
                digits.len() == 1 && digits.bytes().all(|b| b.is_ascii_digit())
            })
        };
        if numbered("Q") || numbered("IQ") {
            return !rest.is_empty() && rest.iter().all(alnum);
        }
        if numbered("MXFP") {
            return rest.iter().all(alnum);
        }
        matches!(head, "BF16" | "F16" | "F32") && rest.is_empty()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_quantization_is_read_from_a_file_name() {
            assert_eq!(
                quantization_of("gemma-3-4b-it-Q4_K_M.gguf").as_deref(),
                Some("Q4_K_M")
            );
            assert_eq!(
                quantization_of("sub/Model.IQ2_XXS.gguf").as_deref(),
                Some("IQ2_XXS")
            );
            assert_eq!(quantization_of("model-BF16.gguf").as_deref(), Some("BF16"));
            assert_eq!(
                quantization_of("large-Q4_K_M-00002-of-00003.gguf").as_deref(),
                Some("Q4_K_M")
            );
            assert_eq!(
                quantization_of("model-UD-Q4_K_XL.gguf").as_deref(),
                Some("UD-Q4_K_XL")
            );
            assert_eq!(quantization_of("mmproj-F16.gguf"), None);
            assert_eq!(quantization_of("README.md"), None);
            assert_eq!(quantization_of("plain-model.gguf"), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn router_model(value: Value) -> RouterModel {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_server_url_is_reduced_to_its_root() {
        assert_eq!(
            normalize_server_url("http://127.0.0.1:8080/v1/").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_server_url(" https://llama.local/?x=1 ").unwrap(),
            "https://llama.local"
        );
        assert!(normalize_server_url("ftp://x").is_err());
        assert!(normalize_server_url("nothing").is_err());
    }

    /// What a model runs with beats what it was started with, which beats what an earlier run saw,
    /// which beats what it was trained for.
    #[test]
    fn the_context_window_comes_from_the_best_source_at_hand() {
        let running = router_model(json!({
            "id": "a", "status": { "value": "loaded", "args": ["-c", "8192"] },
            "meta": { "n_ctx": 16384, "n_ctx_train": 131072 },
        }));
        assert_eq!(running.context_window(Some(4096)), 16384);
        assert_eq!(running.runtime_context_window(), Some(16384));

        let started = router_model(json!({
            "id": "a", "status": { "value": "unloaded", "args": ["--ctx-size", "8192"] },
        }));
        assert_eq!(started.context_window(Some(4096)), 8192);
        assert_eq!(started.runtime_context_window(), None);

        let preset = router_model(json!({
            "id": "a", "status": { "value": "unloaded" }, "meta": { "n_ctx_train": 131072 },
        }));
        assert_eq!(preset.context_window(Some(32768)), 32768);
        assert_eq!(preset.context_window(None), 131072);
    }

    #[test]
    fn only_models_a_request_can_reach_are_selectable() {
        let model = |status: &str, source: &str| {
            router_model(json!({ "id": "a", "status": { "value": status }, "source": source }))
        };
        assert!(model("loaded", "dir").is_selectable(false));
        assert!(model("sleeping", "dir").is_selectable(false));
        assert!(!model("unloaded", "dir").is_selectable(true));
        assert!(!model("unloaded", "preset").is_selectable(false));
        assert!(model("unloaded", "preset").is_selectable(true));
        assert!(!model("loading", "dir").is_selectable(true));
    }

    #[test]
    fn a_router_model_is_a_chat_model_and_a_classifier() {
        let model = router_model(json!({
            "id": "qwen3", "status": { "value": "loaded" },
            "architecture": { "input_modalities": ["text", "image"] },
            "meta": { "n_ctx": 32768 },
        }));
        let props = RouterProps {
            models_autoload: None,
            chat_template: Some("{% if enable_thinking %}".into()),
        };
        let chat = chat_model(&model, "http://127.0.0.1:8080", Some(&props), None);
        assert_eq!(chat.base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(chat.context_window, 32768);
        assert!(chat.reasoning);
        assert_eq!(chat.input, vec![Modality::Text, Modality::Image]);
        assert_eq!(chat.kind(), micro_models::ModelType::Chat);

        let classifier = classifier_model(&model, "http://127.0.0.1:8080", None);
        assert_eq!(classifier.kind(), micro_models::ModelType::Classifier);
        assert_eq!(classifier.base_url, "http://127.0.0.1:8080");
    }

    #[test]
    fn download_progress_adds_up_every_file() {
        let model = router_model(json!({
            "id": "a",
            "status": { "value": "downloading", "progress": {
                "a.gguf": { "done": 512, "total": 1024 },
                "b.gguf": { "done": 0, "total": 1024 },
            }},
        }));
        assert_eq!(model.download_progress(), Some((512, 2048)));
        assert_eq!(format_bytes(2048), "2.00 KiB");
        assert_eq!(format_bytes(20 * 1024 * 1024), "20.0 MiB");
    }

    #[test]
    fn running_context_windows_are_remembered_between_runs() {
        let dir = std::env::temp_dir().join(format!("micro-llama-context-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let loaded = router_model(json!({
            "id": "a", "status": { "value": "loaded" }, "meta": { "n_ctx": 8192 },
        }));
        let unloaded = router_model(json!({ "id": "b", "status": { "value": "unloaded" } }));
        remember_context_windows(&dir, &[loaded, unloaded]).unwrap();

        let remembered = remembered_context_windows(&dir);
        assert_eq!(remembered.get("a"), Some(&8192));
        assert_eq!(remembered.get("b"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
