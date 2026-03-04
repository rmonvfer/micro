use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::cost::{RequestCost, TokenUsage};
use crate::error::{Error, Result};

/// Providers are listed in this order wherever the catalog is presented as a
/// whole. Anything unlisted sorts after these, alphabetically.
const PROVIDER_ORDER: &[&str] = &[
    "openrouter",
    "github-copilot",
    "google",
    "anthropic",
    "openai-codex",
];

/// The wire protocol a model speaks. A single provider often serves several —
/// GitHub Copilot answers Claude models over the Anthropic Messages shape and
/// GPT models over the OpenAI Responses shape — so this is a per-model property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireApi {
    AnthropicMessages,
    OpenaiCompletions,
    OpenaiResponses,
    GoogleGenerativeAi,
}

/// A kind of content a model accepts as input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    Pdf,
}

/// Prices in US dollars per million tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
}

impl ModelCost {
    /// Price a single request. Token counts are taken as reported by the
    /// provider, so `usage.input` must already exclude anything counted under
    /// `cache_read` or `cache_write`.
    pub fn price(&self, usage: TokenUsage) -> RequestCost {
        const PER_MILLION: f64 = 1_000_000.0;
        let scale = |tokens: u64, rate: f64| (tokens as f64 / PER_MILLION) * rate;
        RequestCost {
            input: scale(usage.input, self.input),
            output: scale(usage.output, self.output),
            cache_read: scale(usage.cache_read, self.cache_read),
            cache_write: scale(usage.cache_write, self.cache_write),
        }
    }

    /// Whether any price is set. Subscription-backed providers report zeroes.
    pub fn is_free(&self) -> bool {
        self.input == 0.0 && self.output == 0.0 && self.cache_read == 0.0 && self.cache_write == 0.0
    }
}

/// A model plus everything needed to call it and to price the call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelDef {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api: WireApi,
    pub base_url: String,
    pub context_window: u32,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub input: Vec<Modality>,
    /// Extra headers this model requires, on top of authentication.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Short names a user may type instead of the full id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub cost: ModelCost,
}

impl ModelDef {
    /// The `provider/id` form, which is unique across the catalog.
    pub fn qualified_id(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }

    pub fn accepts(&self, modality: Modality) -> bool {
        self.input.contains(&modality)
    }

    /// Price a single request against this model.
    pub fn price(&self, usage: TokenUsage) -> RequestCost {
        self.cost.price(usage)
    }

    /// The runtime handle a provider needs in order to issue a request.
    pub fn to_runtime(&self, thinking: micro_types::ThinkingLevel) -> micro_types::Model {
        micro_types::Model {
            id: self.id.clone(),
            provider: self.provider.clone(),
            base_url: self.base_url.clone(),
            max_tokens: self.max_output_tokens,
            thinking,
        }
    }
}

impl From<&ModelDef> for micro_types::Model {
    fn from(def: &ModelDef) -> Self {
        def.to_runtime(micro_types::ThinkingLevel::Off)
    }
}

/// The set of models the agent can choose from.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    models: Vec<ModelDef>,
}

impl Catalog {
    /// The catalog compiled into the binary. Always available, no network, no
    /// configuration.
    pub fn bundled() -> Self {
        Self::from_json(crate::bundled::CATALOG_JSON)
            .expect("the bundled catalog is validated by the test suite")
    }

    /// Parse a catalog document.
    pub fn from_json(json: &str) -> Result<Self> {
        let file: CatalogFile = serde_json::from_str(json)?;
        let mut catalog = Catalog::default();
        catalog.apply(file)?;
        Ok(catalog)
    }

    /// A catalog holding exactly these models, for a caller that has already decided
    /// which ones a workspace may use.
    pub fn from_models(models: Vec<ModelDef>) -> Self {
        Catalog { models }
    }

    pub fn models(&self) -> &[ModelDef] {
        &self.models
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    pub fn get(&self, provider: &str, id: &str) -> Option<&ModelDef> {
        self.models
            .iter()
            .find(|m| m.provider == provider && m.id == id)
    }

    /// Provider ids in presentation order.
    pub fn providers(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for model in &self.models {
            if !seen.contains(&model.provider.as_str()) {
                seen.push(&model.provider);
            }
        }
        seen
    }

    pub fn by_provider<'a>(&'a self, provider: &'a str) -> impl Iterator<Item = &'a ModelDef> {
        self.models.iter().filter(move |m| m.provider == provider)
    }

    /// Drop every model whose provider is not in `keep` — the way a caller
    /// narrows the catalog to the providers it actually has credentials for.
    pub fn retain_providers(&mut self, keep: &[&str]) {
        self.models.retain(|m| keep.contains(&m.provider.as_str()));
    }

    /// Insert a model, replacing any existing entry with the same
    /// `provider`/`id`.
    pub fn upsert(&mut self, model: ModelDef) {
        match self
            .models
            .iter()
            .position(|m| m.provider == model.provider && m.id == model.id)
        {
            Some(index) => self.models[index] = model,
            None => self.models.push(model),
        }
        self.sort();
    }

    /// Merge models discovered from a live provider listing.
    ///
    /// A listing is authoritative about what it states and silent about the
    /// rest, so anything it leaves out — headers, aliases, prices a
    /// subscription provider does not quote, limits it omits — is carried over
    /// from what is already known rather than overwritten with a blank.
    pub fn merge_listing(&mut self, listing: impl IntoIterator<Item = ModelDef>) {
        for mut incoming in listing {
            if let Some(existing) = self.get(&incoming.provider, &incoming.id) {
                if incoming.headers.is_empty() {
                    incoming.headers = existing.headers.clone();
                }
                if incoming.aliases.is_empty() {
                    incoming.aliases = existing.aliases.clone();
                }
                if incoming.cost.is_free() {
                    incoming.cost = existing.cost;
                }
                if incoming.context_window == UNKNOWN_LIMIT {
                    incoming.context_window = existing.context_window;
                }
                if incoming.max_output_tokens == UNKNOWN_LIMIT {
                    incoming.max_output_tokens = existing.max_output_tokens;
                }
            }
            if incoming.context_window == UNKNOWN_LIMIT {
                incoming.context_window = DEFAULT_CONTEXT_WINDOW;
            }
            if incoming.max_output_tokens == UNKNOWN_LIMIT {
                incoming.max_output_tokens = DEFAULT_MAX_OUTPUT_TOKENS;
            }

            match self
                .models
                .iter()
                .position(|m| m.provider == incoming.provider && m.id == incoming.id)
            {
                Some(index) => self.models[index] = incoming,
                None => self.models.push(incoming),
            }
        }
        self.sort();
    }

    /// Apply a user catalog document over this catalog: provider-level settings
    /// re-point existing models, and model entries either patch a known model or
    /// register a new one.
    pub fn apply_overrides(&mut self, json: &str) -> Result<()> {
        let file: CatalogFile = serde_json::from_str(json)?;
        self.apply(file)
    }

    fn apply(&mut self, file: CatalogFile) -> Result<()> {
        for (provider, entry) in file.providers {
            self.apply_provider(&provider, entry)?;
        }
        self.sort();
        Ok(())
    }

    fn apply_provider(&mut self, provider: &str, entry: ProviderEntry) -> Result<()> {
        // Provider-level settings re-point every model already registered under
        // this provider — the way a user moves a whole provider to a proxy.
        for model in self.models.iter_mut().filter(|m| m.provider == provider) {
            if let Some(base_url) = &entry.base_url {
                model.base_url = base_url.clone();
            }
            if let Some(api) = entry.api {
                model.api = api;
            }
            model.headers.extend(entry.headers.clone());
        }

        let defaults = self.provider_defaults(provider, &entry);
        for model in entry.models {
            self.apply_model(provider, &defaults, model)?;
        }
        Ok(())
    }

    /// The endpoint settings a new model inherits: whatever the document
    /// declares, falling back to what the provider's existing models use.
    fn provider_defaults(&self, provider: &str, entry: &ProviderEntry) -> ProviderDefaults {
        let existing = self.models.iter().find(|m| m.provider == provider);
        let mut headers = existing.map(|m| m.headers.clone()).unwrap_or_default();
        headers.extend(entry.headers.clone());
        ProviderDefaults {
            base_url: entry
                .base_url
                .clone()
                .or_else(|| existing.map(|m| m.base_url.clone())),
            api: entry.api.or_else(|| existing.map(|m| m.api)),
            headers,
        }
    }

    fn apply_model(
        &mut self,
        provider: &str,
        defaults: &ProviderDefaults,
        entry: ModelEntry,
    ) -> Result<()> {
        if let Some(index) = self
            .models
            .iter()
            .position(|m| m.provider == provider && m.id == entry.id)
        {
            let model = &mut self.models[index];
            if let Some(name) = entry.name {
                model.name = name;
            }
            if let Some(api) = entry.api {
                model.api = api;
            }
            if let Some(base_url) = entry.base_url {
                model.base_url = base_url;
            }
            if let Some(context_window) = entry.context_window {
                model.context_window = context_window;
            }
            if let Some(max_output_tokens) = entry.max_output_tokens {
                model.max_output_tokens = max_output_tokens;
            }
            if let Some(reasoning) = entry.reasoning {
                model.reasoning = reasoning;
            }
            if let Some(input) = entry.input {
                model.input = input;
            }
            if let Some(cost) = entry.cost {
                model.cost = cost;
            }
            if let Some(aliases) = entry.aliases {
                model.aliases = aliases;
            }
            model.headers.extend(entry.headers);
            return Ok(());
        }

        let (Some(api), Some(base_url)) = (
            entry.api.or(defaults.api),
            entry.base_url.clone().or_else(|| defaults.base_url.clone()),
        ) else {
            return Err(Error::IncompleteModel {
                provider: provider.to_string(),
                id: entry.id,
            });
        };

        let mut headers = defaults.headers.clone();
        headers.extend(entry.headers);

        self.models.push(ModelDef {
            name: entry.name.unwrap_or_else(|| entry.id.clone()),
            id: entry.id,
            provider: provider.to_string(),
            api,
            base_url,
            context_window: entry.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
            max_output_tokens: entry.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
            reasoning: entry.reasoning.unwrap_or(false),
            input: entry.input.unwrap_or_else(|| vec![Modality::Text]),
            headers,
            aliases: entry.aliases.unwrap_or_default(),
            cost: entry.cost.unwrap_or_default(),
        });
        Ok(())
    }

    /// Order by provider priority, then by the order models were registered
    /// within a provider, so listings and resolution are reproducible.
    fn sort(&mut self) {
        let rank = |provider: &str| {
            PROVIDER_ORDER
                .iter()
                .position(|p| *p == provider)
                .unwrap_or(PROVIDER_ORDER.len())
        };
        self.models.sort_by(|a, b| {
            rank(&a.provider)
                .cmp(&rank(&b.provider))
                .then_with(|| a.provider.cmp(&b.provider))
        });
    }
}

const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 16_384;

/// A limit a provider listing did not state. [`Catalog::merge_listing`] fills
/// these in rather than overwriting a known value with a guess.
pub(crate) const UNKNOWN_LIMIT: u32 = 0;

struct ProviderDefaults {
    base_url: Option<String>,
    api: Option<WireApi>,
    headers: BTreeMap<String, String>,
}

/// On-disk catalog format, shared by the bundled catalog and user overrides.
#[derive(Debug, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    providers: BTreeMap<String, ProviderEntry>,
}

#[derive(Debug, Deserialize)]
struct ProviderEntry {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api: Option<WireApi>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    models: Vec<ModelEntry>,
}

/// A model as written in a catalog document. Every field but `id` is optional:
/// omitted fields are inherited from the provider, or left untouched when the
/// entry patches a model that already exists.
#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    api: Option<WireApi>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default)]
    input: Option<Vec<Modality>>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    aliases: Option<Vec<String>>,
    #[serde(default)]
    cost: Option<ModelCost>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_parses() {
        let catalog = Catalog::bundled();
        assert!(!catalog.is_empty());
    }

    #[test]
    fn bundled_catalog_covers_the_providers_that_matter() {
        let catalog = Catalog::bundled();
        for provider in ["openrouter", "github-copilot", "google", "anthropic"] {
            assert!(
                catalog.by_provider(provider).next().is_some(),
                "no models for {provider}"
            );
        }
    }

    #[test]
    fn bundled_catalog_has_the_current_claude_family() {
        let catalog = Catalog::bundled();
        for id in [
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5",
            "claude-haiku-4-5-20251001",
        ] {
            assert!(
                catalog.get("anthropic", id).is_some(),
                "anthropic/{id} missing"
            );
        }
    }

    #[test]
    fn bundled_catalog_has_no_duplicate_models() {
        let catalog = Catalog::bundled();
        let mut seen = Vec::new();
        for model in catalog.models() {
            let key = model.qualified_id();
            assert!(!seen.contains(&key), "duplicate entry {key}");
            seen.push(key);
        }
    }

    #[test]
    fn copilot_models_carry_the_editor_headers() {
        let catalog = Catalog::bundled();
        for model in catalog.by_provider("github-copilot") {
            assert_eq!(
                model
                    .headers
                    .get("Copilot-Integration-Id")
                    .map(String::as_str),
                Some("vscode-chat"),
                "{} is missing the Copilot integration header",
                model.qualified_id()
            );
        }
    }

    #[test]
    fn copilot_serves_claude_over_the_anthropic_wire_shape() {
        let catalog = Catalog::bundled();
        let opus = catalog.get("github-copilot", "claude-opus-5").unwrap();
        assert_eq!(opus.api, WireApi::AnthropicMessages);
        let gpt = catalog.get("github-copilot", "gpt-5.6-terra").unwrap();
        assert_eq!(gpt.api, WireApi::OpenaiResponses);
    }

    #[test]
    fn providers_are_listed_in_priority_order() {
        let catalog = Catalog::bundled();
        assert_eq!(
            catalog.providers(),
            vec![
                "openrouter",
                "github-copilot",
                "google",
                "anthropic",
                "openai-codex"
            ]
        );
    }

    #[test]
    fn models_inherit_provider_settings() {
        let catalog = Catalog::from_json(
            r#"{"providers": {"local": {
                "base_url": "http://localhost:11434/v1",
                "api": "openai-completions",
                "headers": {"X-Trace": "on"},
                "models": [{"id": "qwen3-coder"}]
            }}}"#,
        )
        .unwrap();

        let model = catalog.get("local", "qwen3-coder").unwrap();
        assert_eq!(model.base_url, "http://localhost:11434/v1");
        assert_eq!(model.api, WireApi::OpenaiCompletions);
        assert_eq!(model.headers.get("X-Trace").map(String::as_str), Some("on"));
        assert_eq!(model.name, "qwen3-coder");
        assert_eq!(model.input, vec![Modality::Text]);
    }

    #[test]
    fn a_model_may_override_its_provider_settings() {
        let catalog = Catalog::from_json(
            r#"{"providers": {"mixed": {
                "base_url": "https://a.example/v1",
                "api": "openai-completions",
                "models": [
                    {"id": "plain"},
                    {"id": "special", "api": "anthropic-messages", "base_url": "https://b.example"}
                ]
            }}}"#,
        )
        .unwrap();

        assert_eq!(
            catalog.get("mixed", "plain").unwrap().api,
            WireApi::OpenaiCompletions
        );
        let special = catalog.get("mixed", "special").unwrap();
        assert_eq!(special.api, WireApi::AnthropicMessages);
        assert_eq!(special.base_url, "https://b.example");
    }

    #[test]
    fn a_model_without_endpoint_details_is_rejected() {
        let error = Catalog::from_json(r#"{"providers": {"mystery": {"models": [{"id": "x"}]}}}"#)
            .unwrap_err();
        assert!(
            matches!(error, Error::IncompleteModel { ref provider, ref id } if provider == "mystery" && id == "x"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn overrides_patch_only_the_fields_they_name() {
        let mut catalog = Catalog::bundled();
        let before = catalog.get("anthropic", "claude-opus-5").unwrap().clone();

        catalog
            .apply_overrides(
                r#"{"providers": {"anthropic": {"models": [
                    {"id": "claude-opus-5", "max_output_tokens": 32000}
                ]}}}"#,
            )
            .unwrap();

        let after = catalog.get("anthropic", "claude-opus-5").unwrap();
        assert_eq!(after.max_output_tokens, 32_000);
        assert_eq!(after.context_window, before.context_window);
        assert_eq!(after.cost, before.cost);
        assert_eq!(after.name, before.name);
    }

    #[test]
    fn overrides_repoint_a_whole_provider() {
        let mut catalog = Catalog::bundled();
        catalog
            .apply_overrides(
                r#"{"providers": {"anthropic": {
                    "base_url": "https://proxy.internal/anthropic",
                    "headers": {"X-Org": "acme"}
                }}}"#,
            )
            .unwrap();

        for model in catalog.by_provider("anthropic") {
            assert_eq!(model.base_url, "https://proxy.internal/anthropic");
            assert_eq!(model.headers.get("X-Org").map(String::as_str), Some("acme"));
        }
    }

    #[test]
    fn overrides_add_a_model_to_a_known_provider() {
        let mut catalog = Catalog::bundled();
        catalog
            .apply_overrides(
                r#"{"providers": {"anthropic": {"models": [
                    {"id": "claude-experimental", "context_window": 500000}
                ]}}}"#,
            )
            .unwrap();

        let added = catalog.get("anthropic", "claude-experimental").unwrap();
        assert_eq!(added.base_url, "https://api.anthropic.com/v1");
        assert_eq!(added.api, WireApi::AnthropicMessages);
        assert_eq!(added.context_window, 500_000);
    }

    #[test]
    fn overrides_register_a_brand_new_provider() {
        let mut catalog = Catalog::bundled();
        catalog
            .apply_overrides(
                r#"{"providers": {"ollama": {
                    "base_url": "http://localhost:11434/v1",
                    "api": "openai-completions",
                    "models": [{"id": "qwen3-coder:30b", "name": "Qwen3 Coder 30B"}]
                }}}"#,
            )
            .unwrap();

        let model = catalog.get("ollama", "qwen3-coder:30b").unwrap();
        assert_eq!(model.name, "Qwen3 Coder 30B");
        assert!(model.cost.is_free());
        // An unranked provider sorts after the known ones.
        assert_eq!(*catalog.providers().last().unwrap(), "ollama");
    }

    #[test]
    fn merging_a_listing_keeps_local_headers_and_aliases() {
        let mut catalog = Catalog::bundled();
        let fresh = ModelDef {
            id: "claude-opus-5".into(),
            name: "Claude Opus 5".into(),
            provider: "github-copilot".into(),
            api: WireApi::AnthropicMessages,
            base_url: "https://api.individual.githubcopilot.com".into(),
            context_window: 2_000_000,
            max_output_tokens: 64_000,
            reasoning: true,
            input: vec![Modality::Text],
            headers: BTreeMap::new(),
            aliases: Vec::new(),
            cost: ModelCost::default(),
        };

        catalog.merge_listing([fresh]);

        let merged = catalog.get("github-copilot", "claude-opus-5").unwrap();
        assert_eq!(merged.context_window, 2_000_000);
        assert!(merged.headers.contains_key("Copilot-Integration-Id"));
    }

    #[test]
    fn merging_a_listing_adds_unknown_models() {
        let mut catalog = Catalog::bundled();
        let before = catalog.len();
        catalog.merge_listing([ModelDef {
            id: "brand-new".into(),
            name: "Brand New".into(),
            provider: "openrouter".into(),
            api: WireApi::OpenaiCompletions,
            base_url: "https://openrouter.ai/api/v1".into(),
            context_window: 128_000,
            max_output_tokens: 8_192,
            reasoning: false,
            input: vec![Modality::Text],
            headers: BTreeMap::new(),
            aliases: Vec::new(),
            cost: ModelCost::default(),
        }]);
        assert_eq!(catalog.len(), before + 1);
        assert!(catalog.get("openrouter", "brand-new").is_some());
    }

    #[test]
    fn retain_providers_narrows_the_catalog() {
        let mut catalog = Catalog::bundled();
        catalog.retain_providers(&["openrouter", "github-copilot"]);
        assert_eq!(catalog.providers(), vec!["openrouter", "github-copilot"]);
    }

    #[test]
    fn converts_to_a_runtime_model() {
        let catalog = Catalog::bundled();
        let def = catalog.get("anthropic", "claude-opus-5").unwrap();

        let runtime = def.to_runtime(micro_types::ThinkingLevel::High);
        assert_eq!(runtime.id, "claude-opus-5");
        assert_eq!(runtime.provider, "anthropic");
        assert_eq!(runtime.base_url, "https://api.anthropic.com/v1");
        assert_eq!(runtime.max_tokens, def.max_output_tokens);
        assert_eq!(runtime.thinking, micro_types::ThinkingLevel::High);

        let default: micro_types::Model = def.into();
        assert_eq!(default.thinking, micro_types::ThinkingLevel::Off);
    }

    #[test]
    fn model_defs_round_trip_through_json() {
        let catalog = Catalog::bundled();
        let def = catalog.get("github-copilot", "claude-opus-5").unwrap();
        let json = serde_json::to_string(def).unwrap();
        let back: ModelDef = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, def);
    }
}
