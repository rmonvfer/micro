//! The services micro talks to, so a UI can offer them and a caller can switch between them without
//! naming a concrete type.

use crate::Anthropic;
use crate::ApiKey;
use crate::Codex;
use crate::Gemini;
use crate::OpenAi;
use crate::Provider;
use micro_auth::canonical_provider;
use micro_auth::AuthError;
use micro_auth::AuthMethod;
use micro_auth::AuthStore;
use micro_models::ModelDef;
use micro_models::WireApi;
use std::sync::Arc;

/// A service and how it is reached.
pub struct ProviderInfo {
    /// The canonical id, which is also the key its credential is stored under.
    pub id: &'static str,
    /// The name to show a person.
    pub label: &'static str,
    pub auth: AuthMethod,
}

/// Every provider, in the order a picker should show them.
pub fn known_providers() -> &'static [ProviderInfo] {
    static REGISTRY: std::sync::OnceLock<Vec<ProviderInfo>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        micro_auth::provider_table()
            .iter()
            .map(|entry| ProviderInfo {
                id: entry.id.as_str(),
                label: entry.name.as_str(),
                auth: micro_auth::auth_method(&entry.id),
            })
            .collect()
    })
}

/// Look a provider up by any name it answers to.
pub fn provider_info(name: &str) -> Option<&'static ProviderInfo> {
    let id = canonical_provider(name);
    known_providers().iter().find(|info| info.id == id)
}

/// A client that speaks one wire protocol on behalf of one service.
pub fn client_for(api: WireApi, provider: &str) -> Arc<dyn Provider> {
    match api {
        WireApi::AnthropicMessages => Arc::new(Anthropic::new()),
        WireApi::GoogleGenerativeAi => Arc::new(Gemini::new()),
        WireApi::BedrockConverseStream => Arc::new(crate::bedrock::Bedrock::new()),
        WireApi::GoogleVertex => Arc::new(Gemini::vertex()),
        
        WireApi::OpenaiResponses if canonical_provider(provider) == micro_auth::OPENAI_CODEX => {
            Arc::new(Codex::new())
        }
        
        WireApi::OpenaiResponses
            if canonical_provider(provider) == crate::codex::AZURE_PROVIDER =>
        {
            Arc::new(Codex::azure())
        }
        
        WireApi::OpenaiResponses => Arc::new(Codex::for_provider(canonical_provider(provider))),
        WireApi::OpenaiCompletions => Arc::new(OpenAi::for_provider(canonical_provider(provider))),
    }
}

/// The client that serves one model.
pub fn client_for_model(model: &ModelDef) -> Arc<dyn Provider> {
    client_for(model.api, &model.provider)
}

/// A provider ready to stream, and the credential to hand [`Provider::stream`].
pub struct ResolvedProvider {
    pub client: Arc<dyn Provider>,
    pub api_key: ApiKey,
    /// Where this credential's account is served, when it is somewhere other than what the catalog
    /// records.
    pub base_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error(transparent)]
    Auth(#[from] AuthError),
}

/// Pick the client that serves a model and the credential it needs, exchanging an expired token on
/// the way.
pub async fn resolve(
    store: &Arc<AuthStore>,
    model: &ModelDef,
) -> Result<ResolvedProvider, ResolveError> {
    let credential = store.resolve(&model.provider).await?;
    let token = credential.token().to_string();

    
    let base_url = match canonical_provider(&model.provider) == micro_auth::GITHUB_COPILOT {
        true => micro_auth::copilot::base_url_from_token(&token),
        false => None,
    };

    Ok(ResolvedProvider {
        client: client_for_model(model),
        api_key: ApiKey::Stored {
            store: Arc::clone(store),
            provider: model.provider.clone(),
            resolved: token,
        },
        base_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: &str, api: WireApi) -> ModelDef {
        let catalog = micro_models::Catalog::from_json(&format!(
            r#"{{"providers": {{"{provider}": {{
                "base_url": "https://example.test",
                "api": "{}",
                "models": [{{"id": "a-model"}}]
            }}}}}}"#,
            match api {
                WireApi::AnthropicMessages => "anthropic-messages",
                WireApi::GoogleGenerativeAi => "google-generative-ai",
                WireApi::OpenaiResponses => "openai-responses",
                WireApi::OpenaiCompletions => "openai-completions",
                WireApi::BedrockConverseStream => "bedrock-converse-stream",
                WireApi::GoogleVertex => "google-vertex",
            }
        ))
        .unwrap();
        catalog.get(provider, "a-model").unwrap().clone()
    }

    /// One service, two protocols: the client follows the model, not the provider.
    #[test]
    fn a_client_is_built_for_the_protocol_a_model_speaks() {
        let copilot = "github-copilot";
        assert_eq!(
            client_for_model(&model(copilot, WireApi::AnthropicMessages)).name(),
            "anthropic"
        );
        
        assert_eq!(
            client_for_model(&model(copilot, WireApi::OpenaiResponses)).name(),
            copilot
        );
        assert_eq!(
            client_for_model(&model(copilot, WireApi::OpenaiCompletions)).name(),
            copilot
        );
        assert_eq!(
            client_for_model(&model("google", WireApi::GoogleGenerativeAi)).name(),
            "gemini"
        );
        assert_eq!(
            client_for_model(&model("amazon-bedrock", WireApi::BedrockConverseStream)).name(),
            "amazon-bedrock"
        );
    }

    /// A subscription-served Responses endpoint is not the platform one.
    #[test]
    fn codex_is_told_apart_from_the_platform_responses_endpoint() {
        assert_eq!(
            client_for_model(&model("openai-codex", WireApi::OpenaiResponses)).name(),
            "openai-codex"
        );
        assert_eq!(
            client_for_model(&model("openai", WireApi::OpenaiResponses)).name(),
            "openai"
        );
    }

    /// A provider micro has no flavour for still speaks the protocol its models declare.
    #[test]
    fn an_unfamiliar_provider_still_gets_a_client() {
        let client = client_for_model(&model("some-proxy", WireApi::OpenaiCompletions));
        assert_eq!(client.name(), "some-proxy");
    }

    #[test]
    fn the_registry_covers_every_provider_that_can_be_authenticated() {
        let ids: Vec<&str> = known_providers().iter().map(|info| info.id).collect();
        assert_eq!(ids, micro_auth::providers());
    }

    #[test]
    fn each_provider_declares_how_it_is_authenticated() {
        let copilot = provider_info("github-copilot").unwrap();
        assert_eq!(copilot.auth, AuthMethod::OAuth);
        assert_eq!(
            provider_info("openrouter").unwrap().auth,
            AuthMethod::ApiKey
        );
    }

    #[test]
    fn a_provider_is_found_by_any_name_it_answers_to() {
        assert_eq!(provider_info("copilot").unwrap().id, "github-copilot");
        assert_eq!(provider_info("gemini").unwrap().id, "google");
        assert_eq!(provider_info("GOOGLE").unwrap().id, "google");
        assert!(provider_info("nowhere").is_none());
    }
}
