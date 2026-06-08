use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

use crate::catalog::{Catalog, Modality, ModelCost, ModelDef, WireApi, UNKNOWN_LIMIT};
use crate::error::{Error, Result};

pub const OPENROUTER_PROVIDER: &str = "openrouter";
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

pub const COPILOT_PROVIDER: &str = "github-copilot";
pub const COPILOT_BASE_URL: &str = "https://api.individual.githubcopilot.com";
const COPILOT_API_VERSION: &str = "2025-04-01";

/// Copilot answers an editor rather than a program, and turns away any request that does not say
/// which editor it is. Every request carries these, listing the models included.
const COPILOT_EDITOR: [(&str, &str); 4] = [
    ("User-Agent", "GitHubCopilotChat/0.35.0"),
    ("Editor-Version", "vscode/1.107.0"),
    ("Editor-Plugin-Version", "copilot-chat/0.35.0"),
    ("Copilot-Integration-Id", "vscode-chat"),
];

/// Those headers, as a model carries them.
fn copilot_editor_headers() -> BTreeMap<String, String> {
    COPILOT_EDITOR
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

const LISTING_TIMEOUT: Duration = Duration::from_secs(10);

/// Fetch OpenRouter's model list.
pub async fn fetch_openrouter(client: &reqwest::Client) -> Result<Vec<ModelDef>> {
    let url = format!("{OPENROUTER_BASE_URL}/models");
    let body = get(client, &url, OPENROUTER_PROVIDER, &[]).await?;
    parse_openrouter(&body)
}

/// Fetch the models the given Copilot token is entitled to.
pub async fn fetch_copilot(
    client: &reqwest::Client,
    token: &str,
    base_url: &str,
) -> Result<Vec<ModelDef>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut headers = vec![
        ("Authorization", format!("Bearer {token}")),
        ("X-GitHub-Api-Version", COPILOT_API_VERSION.to_string()),
    ];
    headers.extend(
        COPILOT_EDITOR
            .iter()
            .map(|(name, value)| (*name, value.to_string())),
    );
    let body = get(client, &url, COPILOT_PROVIDER, &headers).await?;
    parse_copilot(&body, base_url)
}

async fn get(
    client: &reqwest::Client,
    url: &str,
    provider: &'static str,
    headers: &[(&str, String)],
) -> Result<String> {
    let mut request = client
        .get(url)
        .header("Accept", "application/json")
        .timeout(LISTING_TIMEOUT);
    for (name, value) in headers {
        request = request.header(*name, value);
    }

    let response = request.send().await.map_err(|source| Error::Http {
        url: url.to_string(),
        source,
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(Error::ListingStatus {
            provider,
            status: status.as_u16(),
        });
    }

    response.text().await.map_err(|source| Error::Http {
        url: url.to_string(),
        source,
    })
}

/// Parse an OpenRouter `/models` response, keeping the tool-capable models.
pub fn parse_openrouter(body: &str) -> Result<Vec<ModelDef>> {
    let envelope: OpenRouterEnvelope =
        serde_json::from_str(body).map_err(|error| Error::ListingShape {
            provider: OPENROUTER_PROVIDER,
            reason: error.to_string(),
        })?;

    let models = envelope
        .data
        .into_iter()
        .filter(|model| model.supported_parameters.iter().any(|p| p == "tools"))
        .map(|model| {
            let pricing = model.pricing.unwrap_or_default();
            let top = model.top_provider.unwrap_or_default();
            ModelDef {
                name: model.name.unwrap_or_else(|| model.id.clone()),
                id: model.id,
                provider: OPENROUTER_PROVIDER.to_string(),
                api: WireApi::OpenaiCompletions,
                base_url: OPENROUTER_BASE_URL.to_string(),
                context_window: top
                    .context_length
                    .or(model.context_length)
                    .unwrap_or(UNKNOWN_LIMIT),
                max_output_tokens: top.max_completion_tokens.unwrap_or(UNKNOWN_LIMIT),
                reasoning: model.supported_parameters.iter().any(|p| p == "reasoning"),
                input: modalities(
                    model
                        .architecture
                        .unwrap_or_default()
                        .input_modalities
                        .iter()
                        .map(String::as_str),
                ),
                headers: BTreeMap::new(),
                aliases: Vec::new(),
                cost: ModelCost {
                    input: per_million(pricing.prompt.as_deref()),
                    output: per_million(pricing.completion.as_deref()),
                    cache_read: per_million(pricing.input_cache_read.as_deref()),
                    cache_write: per_million(pricing.input_cache_write.as_deref()),
                    tiers: Vec::new(),
                },
                compat: Default::default(),
                thinking: Default::default(),
            }
        })
        .collect();

    Ok(models)
}

/// Parse a GitHub Copilot `/models` response, keeping the chat models the account may actually
/// select.
pub fn parse_copilot(body: &str, base_url: &str) -> Result<Vec<ModelDef>> {
    let envelope: CopilotEnvelope =
        serde_json::from_str(body).map_err(|error| Error::ListingShape {
            provider: COPILOT_PROVIDER,
            reason: error.to_string(),
        })?;

    let base_url = base_url.trim_end_matches('/').to_string();
    let models = envelope
        .data
        .into_iter()
        .filter(is_selectable)
        .map(|model| {
            let capabilities = model.capabilities.unwrap_or_default();
            let limits = capabilities.limits.unwrap_or_default();
            let mut input = vec![Modality::Text];
            if capabilities.supports.vision == Some(true) {
                input.push(Modality::Image);
            }
            ModelDef {
                api: copilot_api(&model.id),
                name: model.name.unwrap_or_else(|| model.id.clone()),
                id: model.id,
                provider: COPILOT_PROVIDER.to_string(),
                base_url: base_url.clone(),
                context_window: limits.max_context_window_tokens.unwrap_or(UNKNOWN_LIMIT),
                max_output_tokens: limits.max_output_tokens.unwrap_or(UNKNOWN_LIMIT),
                reasoning: false,
                input,

                headers: copilot_editor_headers(),
                aliases: Vec::new(),
                cost: ModelCost::default(),
                compat: Default::default(),
                thinking: Default::default(),
            }
        })
        .collect();

    Ok(models)
}

impl Catalog {
    /// Merge live provider listings over the catalog so models released since this build appear
    /// without a release.
    pub async fn merge_live_listings(
        &mut self,
        client: &reqwest::Client,
        copilot: Option<CopilotCredentials<'_>>,
    ) -> Vec<Error> {
        let mut failures = Vec::new();

        match fetch_openrouter(client).await {
            Ok(models) => self.merge_listing(models),
            Err(error) => failures.push(error),
        }

        if let Some(credentials) = copilot {
            match fetch_copilot(client, credentials.token, credentials.base_url).await {
                Ok(models) => self.merge_listing(models),
                Err(error) => failures.push(error),
            }
        }

        failures
    }
}

/// What listing GitHub Copilot's models requires: a Copilot token and, for enterprise accounts, the
/// endpoint that serves them.
#[derive(Debug, Clone, Copy)]
pub struct CopilotCredentials<'a> {
    pub token: &'a str,
    pub base_url: &'a str,
}

impl<'a> CopilotCredentials<'a> {
    pub fn new(token: &'a str) -> Self {
        CopilotCredentials {
            token,
            base_url: COPILOT_BASE_URL,
        }
    }

    pub fn with_base_url(mut self, base_url: &'a str) -> Self {
        self.base_url = base_url;
        self
    }
}

/// The wire a model speaks when the catalog does not list it, for the providers whose models say so
/// in their names. Nothing is assumed of the rest.
pub(crate) fn assumed_api(provider: &str, id: &str) -> Option<WireApi> {
    match provider {
        COPILOT_PROVIDER => Some(copilot_api(id)),
        _ => None,
    }
}

fn copilot_api(id: &str) -> WireApi {
    if id == "claude-fable-5" {
        WireApi::OpenaiCompletions
    } else if id.starts_with("claude-") {
        WireApi::AnthropicMessages
    } else if id.starts_with("gpt-5") || id.starts_with("oswe") {
        WireApi::OpenaiResponses
    } else {
        WireApi::OpenaiCompletions
    }
}

/// A Copilot model is usable when the account can pick it, its terms are not pending acceptance,
/// and it can call tools.
fn is_selectable(model: &CopilotModel) -> bool {
    let capabilities = model.capabilities.as_ref();
    let is_chat = capabilities
        .and_then(|c| c.kind.as_deref())
        .is_none_or(|kind| kind == "chat");
    let tool_calls = capabilities
        .and_then(|c| c.supports.tool_calls)
        .unwrap_or(true);
    let enabled = model
        .policy
        .as_ref()
        .and_then(|p| p.state.as_deref())
        .is_none_or(|state| state != "disabled");

    model.model_picker_enabled && is_chat && tool_calls && enabled
}

/// OpenRouter quotes prices per token as decimal strings; the catalog stores dollars per million
/// tokens.
fn per_million(price: Option<&str>) -> f64 {
    let Some(parsed) = price.and_then(|p| p.parse::<f64>().ok()) else {
        return 0.0;
    };
    if !parsed.is_finite() {
        return 0.0;
    }

    (parsed * 1_000_000.0 * 1e6).round() / 1e6
}

fn modalities<'a>(names: impl Iterator<Item = &'a str>) -> Vec<Modality> {
    let mut input = Vec::new();
    for name in names {
        let modality = match name {
            "text" => Modality::Text,
            "image" => Modality::Image,
            "audio" => Modality::Audio,
            "video" => Modality::Video,
            "file" | "pdf" => Modality::Pdf,
            _ => continue,
        };
        if !input.contains(&modality) {
            input.push(modality);
        }
    }
    if input.is_empty() {
        input.push(Modality::Text);
    }
    input
}

#[derive(Debug, Deserialize)]
struct OpenRouterEnvelope {
    #[serde(default)]
    data: Vec<OpenRouterModel>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    architecture: Option<OpenRouterArchitecture>,
    #[serde(default)]
    pricing: Option<OpenRouterPricing>,
    #[serde(default)]
    top_provider: Option<OpenRouterTopProvider>,
    #[serde(default)]
    supported_parameters: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenRouterArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenRouterPricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
    #[serde(default)]
    input_cache_read: Option<String>,
    #[serde(default)]
    input_cache_write: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenRouterTopProvider {
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    max_completion_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CopilotEnvelope {
    #[serde(default)]
    data: Vec<CopilotModel>,
}

#[derive(Debug, Deserialize)]
struct CopilotModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model_picker_enabled: bool,
    #[serde(default)]
    policy: Option<CopilotPolicy>,
    #[serde(default)]
    capabilities: Option<CopilotCapabilities>,
}

#[derive(Debug, Deserialize)]
struct CopilotPolicy {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CopilotCapabilities {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    limits: Option<CopilotLimits>,
    #[serde(default)]
    supports: CopilotSupports,
}

#[derive(Debug, Default, Deserialize)]
struct CopilotLimits {
    #[serde(default)]
    max_context_window_tokens: Option<u32>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct CopilotSupports {
    #[serde(default)]
    tool_calls: Option<bool>,
    #[serde(default)]
    vision: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENROUTER_SAMPLE: &str = include_str!("../testdata/openrouter-models.json");
    const COPILOT_SAMPLE: &str = include_str!("../testdata/copilot-models.json");

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn parses_an_openrouter_listing() {
        let models = parse_openrouter(OPENROUTER_SAMPLE).unwrap();

        let opus = models
            .iter()
            .find(|m| m.id == "anthropic/claude-opus-5")
            .expect("claude-opus-5 missing from the sample listing");

        assert_eq!(opus.provider, "openrouter");
        assert_eq!(opus.api, WireApi::OpenaiCompletions);
        assert_eq!(opus.base_url, OPENROUTER_BASE_URL);
        assert_eq!(opus.context_window, 1_000_000);
        assert_eq!(opus.max_output_tokens, 128_000);
        assert!(opus.reasoning);
        assert_eq!(
            opus.input,
            vec![Modality::Text, Modality::Image, Modality::Pdf]
        );
    }

    #[test]
    fn converts_openrouter_per_token_prices_to_per_million() {
        let models = parse_openrouter(OPENROUTER_SAMPLE).unwrap();
        let opus = models
            .iter()
            .find(|m| m.id == "anthropic/claude-opus-5")
            .unwrap();

        assert_close(opus.cost.input, 5.0);
        assert_close(opus.cost.output, 25.0);
        assert_close(opus.cost.cache_read, 0.5);
        assert_close(opus.cost.cache_write, 6.25);
    }

    #[test]
    fn a_missing_openrouter_price_reads_as_zero() {
        let models = parse_openrouter(OPENROUTER_SAMPLE).unwrap();
        let glm = models.iter().find(|m| m.id == "z-ai/glm-5.2").unwrap();

        assert_close(glm.cost.cache_write, 0.0);
        assert_close(glm.cost.input, 0.76);
        assert_eq!(glm.input, vec![Modality::Text]);
    }

    #[test]
    fn openrouter_models_without_tool_support_are_dropped() {
        let models = parse_openrouter(OPENROUTER_SAMPLE).unwrap();
        assert!(
            !models
                .iter()
                .any(|m| m.id == "google/gemini-3.1-flash-lite-image"),
            "an image model with no tool support should not be offered to the agent"
        );
        assert_eq!(models.len(), 2);
    }

    #[test]
    fn an_empty_openrouter_listing_parses_to_nothing() {
        assert!(parse_openrouter(r#"{"data": []}"#).unwrap().is_empty());
        assert!(parse_openrouter("{}").unwrap().is_empty());
    }

    #[test]
    fn a_malformed_openrouter_listing_is_reported() {
        let error = parse_openrouter(r#"{"data": "nope"}"#).unwrap_err();
        assert!(
            matches!(error, Error::ListingShape { provider, .. } if provider == OPENROUTER_PROVIDER),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parses_a_copilot_listing() {
        let models = parse_copilot(COPILOT_SAMPLE, COPILOT_BASE_URL).unwrap();

        let opus = models
            .iter()
            .find(|m| m.id == "claude-opus-5")
            .expect("claude-opus-5 missing from the sample listing");

        assert_eq!(opus.provider, "github-copilot");
        assert_eq!(opus.base_url, COPILOT_BASE_URL);
        assert_eq!(opus.context_window, 1_000_000);
        assert_eq!(opus.max_output_tokens, 64_000);
        assert_eq!(opus.input, vec![Modality::Text, Modality::Image]);
    }

    #[test]
    fn copilot_models_get_their_vendor_wire_shape() {
        let models = parse_copilot(COPILOT_SAMPLE, COPILOT_BASE_URL).unwrap();
        let api_of = |id: &str| models.iter().find(|m| m.id == id).unwrap().api;

        assert_eq!(api_of("claude-opus-5"), WireApi::AnthropicMessages);
        assert_eq!(api_of("gpt-5.6-terra"), WireApi::OpenaiResponses);
        assert_eq!(api_of("kimi-k3"), WireApi::OpenaiCompletions);
        assert_eq!(copilot_api("claude-fable-5"), WireApi::OpenaiCompletions);
    }

    #[test]
    fn unusable_copilot_models_are_dropped() {
        let models = parse_copilot(COPILOT_SAMPLE, COPILOT_BASE_URL).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();

        assert!(!ids.contains(&"text-embedding-3-small"), "not a chat model");
        assert!(!ids.contains(&"claude-opus-4.8"), "policy is disabled");
        assert!(!ids.contains(&"gpt-4o-mini-no-tools"), "cannot call tools");
        assert_eq!(ids, vec!["claude-opus-5", "gpt-5.6-terra", "kimi-k3"]);
    }

    #[test]
    fn a_copilot_listing_reports_no_pricing() {
        let models = parse_copilot(COPILOT_SAMPLE, COPILOT_BASE_URL).unwrap();
        assert!(models.iter().all(|m| m.cost.is_free()));
    }

    /// Copilot turns away a request that does not name the editor asking, so a model discovered
    /// from the listing carries the same headers as one that shipped in the catalog. Without this
    /// a model too new to be bundled answers 400 and nothing else.
    #[test]
    fn a_listed_copilot_model_names_the_editor_it_is_for() {
        let models = parse_copilot(COPILOT_SAMPLE, COPILOT_BASE_URL).unwrap();

        assert!(!models.is_empty());
        for model in &models {
            assert_eq!(
                model.headers.get("Editor-Version").map(String::as_str),
                Some("vscode/1.107.0"),
                "{} was listed without an editor",
                model.id
            );
            assert!(model.headers.contains_key("Copilot-Integration-Id"));
            assert!(model.headers.contains_key("Editor-Plugin-Version"));
            assert!(model.headers.contains_key("User-Agent"));
        }
    }

    /// A listed model the catalog has never heard of is the case the bundled headers cannot cover.
    #[test]
    fn a_copilot_model_the_catalog_does_not_know_still_names_the_editor() {
        let mut catalog = Catalog::bundled();
        catalog.merge_listing(parse_copilot(COPILOT_SAMPLE, COPILOT_BASE_URL).unwrap());

        let listed = catalog
            .get("github-copilot", "kimi-k3")
            .expect("the listing added it");
        assert_eq!(
            listed.headers.get("Editor-Version").map(String::as_str),
            Some("vscode/1.107.0")
        );
    }

    #[test]
    fn an_enterprise_copilot_endpoint_is_carried_onto_the_models() {
        let models = parse_copilot(COPILOT_SAMPLE, "https://copilot-api.acme.example/").unwrap();
        assert!(models
            .iter()
            .all(|m| m.base_url == "https://copilot-api.acme.example"));
    }

    #[test]
    fn a_malformed_copilot_listing_is_reported() {
        let error = parse_copilot("not json", COPILOT_BASE_URL).unwrap_err();
        assert!(
            matches!(error, Error::ListingShape { provider, .. } if provider == COPILOT_PROVIDER),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn merging_a_copilot_listing_keeps_bundled_pricing_and_headers() {
        let mut catalog = Catalog::bundled();
        let priced_before = catalog
            .get("github-copilot", "claude-opus-5")
            .unwrap()
            .cost
            .clone();

        catalog.merge_listing(parse_copilot(COPILOT_SAMPLE, COPILOT_BASE_URL).unwrap());

        let merged = catalog.get("github-copilot", "claude-opus-5").unwrap();
        assert_eq!(merged.cost, priced_before);
        assert!(!merged.cost.is_free());
        assert!(merged.headers.contains_key("Copilot-Integration-Id"));
        assert_eq!(merged.api, WireApi::AnthropicMessages);
    }

    #[test]
    fn merging_an_openrouter_listing_updates_pricing() {
        let mut catalog = Catalog::bundled();
        catalog
            .apply_overrides(
                r#"{"providers": {"openrouter": {"models": [
                    {"id": "anthropic/claude-opus-5", "aliases": ["opus"], "cost": {"input": 999.0, "output": 999.0}}
                ]}}}"#,
            )
            .unwrap();

        catalog.merge_listing(parse_openrouter(OPENROUTER_SAMPLE).unwrap());

        let merged = catalog
            .get("openrouter", "anthropic/claude-opus-5")
            .unwrap();
        assert_close(merged.cost.input, 5.0);
        assert_eq!(merged.aliases, vec!["opus".to_string()]);
    }

    #[test]
    fn a_listing_without_limits_keeps_the_bundled_ones() {
        let mut catalog = Catalog::bundled();
        let before = catalog
            .get("openrouter", "anthropic/claude-opus-5")
            .unwrap();
        let (context, output) = (before.context_window, before.max_output_tokens);

        catalog.merge_listing(
            parse_openrouter(
                r#"{"data": [{
                "id": "anthropic/claude-opus-5",
                "name": "Claude Opus 5",
                "supported_parameters": ["tools"],
                "pricing": {"prompt": "0.000005", "completion": "0.000025"}
            }]}"#,
            )
            .unwrap(),
        );

        let merged = catalog
            .get("openrouter", "anthropic/claude-opus-5")
            .unwrap();
        assert_eq!(merged.context_window, context);
        assert_eq!(merged.max_output_tokens, output);
    }

    #[test]
    fn a_new_model_without_limits_gets_workable_defaults() {
        let mut catalog = Catalog::bundled();
        catalog.merge_listing(
            parse_openrouter(
                r#"{"data": [{"id": "vendor/brand-new", "supported_parameters": ["tools"]}]}"#,
            )
            .unwrap(),
        );

        let added = catalog.get("openrouter", "vendor/brand-new").unwrap();
        assert!(added.context_window > 0);
        assert!(added.max_output_tokens > 0);
    }

    #[test]
    fn copilot_credentials_default_to_the_individual_endpoint() {
        let credentials = CopilotCredentials::new("token");
        assert_eq!(credentials.base_url, COPILOT_BASE_URL);

        let enterprise = credentials.with_base_url("https://copilot-api.acme.example");
        assert_eq!(enterprise.base_url, "https://copilot-api.acme.example");
        assert_eq!(enterprise.token, "token");
    }
}
