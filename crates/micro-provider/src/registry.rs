//! The providers micro knows about, so a UI can offer them and a caller can switch
//! between them without naming a concrete type.
//!
//! [`micro_auth`] owns the provider ids and how each one authenticates; this module adds
//! where each one lives and what talks to it.

use crate::Anthropic;
use crate::Codex;
use crate::Gemini;
use crate::OpenAi;
use crate::Provider;
use micro_auth::canonical_provider;
use micro_auth::AuthError;
use micro_auth::AuthMethod;
use micro_auth::AuthStore;
use micro_types::Model;
use micro_types::ThinkingLevel;
use std::sync::Arc;

pub(crate) const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1";

/// A provider, its endpoint, and how to reach it.
pub struct ProviderInfo {
    /// The canonical id, which is also the key its credential is stored under.
    pub id: &'static str,
    /// The name to show a person.
    pub label: &'static str,
    /// Where its models live when a catalog does not say otherwise.
    pub base_url: &'static str,
    pub auth: AuthMethod,
    /// A conservative output limit, used when the model's real one is unknown.
    pub max_tokens: u32,
}

impl ProviderInfo {
    /// A client for this provider. Each call builds one; a caller that streams often
    /// should keep it.
    pub fn client(&self) -> Arc<dyn Provider> {
        match self.id {
            "anthropic" => Arc::new(Anthropic::new()),
            "openrouter" => Arc::new(OpenAi::openrouter()),
            "github-copilot" => Arc::new(OpenAi::copilot()),
            "gemini" => Arc::new(Gemini::new()),
            "openai-codex" => Arc::new(Codex::new()),
            _ => Arc::new(OpenAi::new()),
        }
    }

    /// A model served by this provider at its default endpoint.
    pub fn model(&self, id: impl Into<String>) -> Model {
        self.model_at(id, self.base_url)
    }

    /// A model served by this provider at an endpoint a catalog supplied, which is how a
    /// model that lives behind a proxy or a regional host is reached.
    pub fn model_at(&self, id: impl Into<String>, base_url: impl Into<String>) -> Model {
        Model {
            id: id.into(),
            provider: self.id.to_string(),
            base_url: base_url.into(),
            max_tokens: self.max_tokens,
            thinking: ThinkingLevel::Off,
        }
    }
}

static REGISTRY: &[ProviderInfo] = &[
    ProviderInfo {
        id: "anthropic",
        label: "Anthropic",
        base_url: ANTHROPIC_BASE_URL,
        auth: AuthMethod::ApiKey,
        max_tokens: 32_000,
    },
    ProviderInfo {
        id: "openrouter",
        label: "OpenRouter",
        base_url: crate::openai::OPENROUTER_BASE_URL,
        auth: AuthMethod::ApiKey,
        max_tokens: 32_000,
    },
    ProviderInfo {
        id: "github-copilot",
        label: "GitHub Copilot",
        base_url: crate::openai::COPILOT_BASE_URL,
        auth: AuthMethod::OAuth,
        max_tokens: 16_000,
    },
    ProviderInfo {
        id: "gemini",
        label: "Google Gemini",
        base_url: crate::gemini::GEMINI_BASE_URL,
        auth: AuthMethod::ApiKey,
        max_tokens: 32_000,
    },
    ProviderInfo {
        id: "openai",
        label: "OpenAI",
        base_url: crate::openai::OPENAI_BASE_URL,
        auth: AuthMethod::ApiKey,
        max_tokens: 32_000,
    },
    ProviderInfo {
        id: "openai-codex",
        label: "ChatGPT Codex",
        base_url: crate::codex::CODEX_BASE_URL,
        auth: AuthMethod::ApiKey,
        max_tokens: 128_000,
    },
];

/// Every provider, in the order a picker should show them.
pub fn known_providers() -> &'static [ProviderInfo] {
    REGISTRY
}

/// Look a provider up by any name it answers to.
pub fn provider_info(name: &str) -> Option<&'static ProviderInfo> {
    let id = canonical_provider(name);
    REGISTRY.iter().find(|info| info.id == id)
}

/// Build a provider by name.
pub fn provider_by_name(name: &str) -> Option<Arc<dyn Provider>> {
    provider_info(name).map(ProviderInfo::client)
}

/// A model pointed at the provider that serves it, so a caller only supplies an id.
pub fn model_for(provider: &str, id: impl Into<String>) -> Option<Model> {
    provider_info(provider).map(|info| info.model(id))
}

/// A provider ready to stream, and the credential to hand [`Provider::stream`].
pub struct ResolvedProvider {
    pub info: &'static ProviderInfo,
    pub client: Arc<dyn Provider>,
    pub api_key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("unknown provider `{0}`")]
    Unknown(String),

    #[error(transparent)]
    Auth(#[from] AuthError),
}

/// Pick a provider and its credential in one step, exchanging an expired token on the
/// way. This is what a CLI or TUI calls when the user names a provider.
pub async fn resolve(store: &AuthStore, name: &str) -> Result<ResolvedProvider, ResolveError> {
    let info = provider_info(name).ok_or_else(|| ResolveError::Unknown(name.to_string()))?;
    let credential = store.resolve(info.id).await?;

    Ok(ResolvedProvider {
        info,
        client: info.client(),
        api_key: credential.token().to_string(),
    })
}

/// A client that speaks one wire protocol, for a provider the registry does not know.
///
/// An extension may declare a provider of its own; what matters then is not who serves it
/// but which shape the request takes.
pub fn client_for(api: micro_models::WireApi) -> Arc<dyn Provider> {
    match api {
        micro_models::WireApi::AnthropicMessages => Arc::new(Anthropic::new()),
        micro_models::WireApi::GoogleGenerativeAi => Arc::new(Gemini::new()),
        micro_models::WireApi::OpenaiResponses | micro_models::WireApi::OpenaiCompletions => {
            Arc::new(OpenAi::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_builds_a_client_that_answers_to_its_own_id() {
        for info in known_providers() {
            assert_eq!(info.client().name(), info.id, "{}", info.label);
        }
    }

    #[test]
    fn the_registry_covers_every_provider_that_can_be_authenticated() {
        let ids: Vec<&str> = known_providers().iter().map(|info| info.id).collect();
        assert_eq!(ids, micro_auth::PROVIDERS.to_vec());
    }

    #[test]
    fn a_provider_is_found_by_any_name_it_answers_to() {
        assert_eq!(provider_info("copilot").unwrap().id, "github-copilot");
        assert_eq!(provider_info("google").unwrap().id, "gemini");
        assert_eq!(provider_info("GEMINI").unwrap().id, "gemini");
        assert!(provider_info("nowhere").is_none());
        assert!(provider_by_name("nowhere").is_none());
    }

    #[test]
    fn each_provider_declares_how_it_is_authenticated() {
        assert_eq!(provider_info("copilot").unwrap().auth, AuthMethod::OAuth);
        assert_eq!(provider_info("gemini").unwrap().auth, AuthMethod::ApiKey);
    }

    #[test]
    fn a_model_carries_the_endpoint_of_the_provider_that_serves_it() {
        let model = model_for("openrouter", "anthropic/claude-opus-4").unwrap();
        assert_eq!(model.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(model.provider, "openrouter");
        assert_eq!(model.id, "anthropic/claude-opus-4");

        assert_eq!(
            model_for("anthropic", "claude-opus-5").unwrap().base_url,
            Model::anthropic("claude-opus-5").base_url
        );
        assert!(model_for("nowhere", "x").is_none());
    }

    #[tokio::test]
    async fn resolving_hands_back_the_client_and_the_credential_together() {
        let directory = std::env::temp_dir().join(format!("micro-registry-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let store = AuthStore::open_at(directory.join("auth.json")).unwrap();
        store.store_api_key("openrouter", "sk-or").unwrap();

        let resolved = resolve(&store, "openrouter").await.unwrap();
        assert_eq!(resolved.info.id, "openrouter");
        assert_eq!(resolved.client.name(), "openrouter");
        assert_eq!(resolved.api_key, "sk-or");
    }

    #[tokio::test]
    async fn resolving_a_provider_micro_does_not_know_names_it() {
        let directory = std::env::temp_dir().join(format!("micro-registry-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let store = AuthStore::open_at(directory.join("auth.json")).unwrap();

        match resolve(&store, "nowhere").await {
            Err(ResolveError::Unknown(name)) => assert_eq!(name, "nowhere"),
            Err(other) => panic!("expected an unknown provider, got {other}"),
            Ok(_) => panic!("expected an unknown provider"),
        }
    }

    #[test]
    fn a_catalog_can_override_where_a_model_lives() {
        let model = provider_info("gemini")
            .unwrap()
            .model_at("gemini-2.5-pro", "https://proxy.internal/v1beta");

        assert_eq!(model.base_url, "https://proxy.internal/v1beta");
        assert_eq!(model.provider, "gemini");
    }
}
