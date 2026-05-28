//! What a service accepts, worked out from who serves a model and where.

use micro_types::CacheControlFormat;
use micro_types::Compat;
use micro_types::MaxTokensField;
use micro_types::SessionAffinity;
use micro_types::ThinkingFormat;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;

/// What one catalog entry states about its service, where it differs from the protocol.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_store: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_developer_role: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_reasoning_effort: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_usage_in_streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_field: Option<MaxTokensField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_reasoning_content_on_assistant_messages: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_format: Option<ThinkingFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zai_tool_stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_strict_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_strict_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bedrock_supports_strict_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control_format: Option<CacheControlFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_session_affinity_headers: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_affinity_format: Option<SessionAffinity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_long_cache_retention: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_temperature: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_eager_tool_input_streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_cache_control_on_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_adaptive_thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id_length: Option<usize>,
    /// Whether the service offers the model a search over its own tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tool_search: Option<bool>,
}

impl CompatOverrides {
    /// Whether this entry states anything at all.
    pub fn is_empty(&self) -> bool {
        *self == CompatOverrides::default()
    }
}

/// Everything decided about how a model's service is spoken to.
pub fn resolve(
    provider: &str,
    base_url: &str,
    model_id: &str,
    stated: &CompatOverrides,
    thinking: &BTreeMap<String, Option<String>>,
) -> Compat {
    let mut compat = detect(provider, base_url, model_id);

    if let Some(value) = stated.supports_store {
        compat.supports_store = value;
    }
    if let Some(value) = stated.supports_developer_role {
        compat.supports_developer_role = value;
    }
    if let Some(value) = stated.supports_reasoning_effort {
        compat.supports_reasoning_effort = value;
    }
    if let Some(value) = stated.supports_usage_in_streaming {
        compat.supports_usage_in_streaming = value;
    }
    if let Some(value) = stated.max_tokens_field {
        compat.max_tokens_field = value;
    }
    if let Some(value) = stated.requires_reasoning_content_on_assistant_messages {
        compat.requires_reasoning_content_on_assistant_messages = value;
    }
    if let Some(value) = stated.thinking_format {
        compat.thinking_format = value;
    }
    if let Some(value) = stated.zai_tool_stream {
        compat.zai_tool_stream = value;
    }
    if let Some(value) = stated.supports_strict_mode {
        compat.supports_strict_mode = value;
    }
    if let Some(value) = stated.supports_strict_tools {
        compat.supports_strict_tools = value;
    }
    if let Some(value) = stated.bedrock_supports_strict_tools {
        compat.bedrock_supports_strict_tools = value;
    }
    if let Some(value) = stated.cache_control_format {
        compat.cache_control_format = Some(value);
    }
    if let Some(value) = stated.send_session_affinity_headers {
        compat.send_session_affinity_headers = value;
    }
    if let Some(value) = stated.session_affinity_format {
        compat.session_affinity_format = value;
    }
    if let Some(value) = stated.supports_long_cache_retention {
        compat.supports_long_cache_retention = value;
    }
    if let Some(value) = stated.supports_temperature {
        compat.supports_temperature = value;
    }
    if let Some(value) = stated.supports_eager_tool_input_streaming {
        compat.supports_eager_tool_input_streaming = value;
    }
    if let Some(value) = stated.supports_cache_control_on_tools {
        compat.supports_cache_control_on_tools = value;
    }
    if let Some(value) = stated.force_adaptive_thinking {
        compat.force_adaptive_thinking = value;
    }
    if let Some(value) = stated.tool_call_id_length {
        compat.tool_call_id_length = Some(value);
    }

    compat.thinking = thinking.clone();
    compat
}

/// What a service is assumed to accept, going by who it is and where it lives.
fn detect(provider: &str, base_url: &str, model_id: &str) -> Compat {
    let is_zai = provider == "zai"
        || provider == "zai-coding-cn"
        || base_url.contains("api.z.ai")
        || base_url.contains("open.bigmodel.cn");
    let is_together = provider == "together"
        || base_url.contains("api.together.ai")
        || base_url.contains("api.together.xyz");
    let is_moonshot = provider == "moonshotai"
        || provider == "moonshotai-cn"
        || base_url.contains("api.moonshot.");
    let is_openrouter = provider == "openrouter" || base_url.contains("openrouter.ai");
    let is_cloudflare_workers =
        provider == "cloudflare-workers-ai" || base_url.contains("api.cloudflare.com");
    let is_cloudflare_gateway =
        provider == "cloudflare-ai-gateway" || base_url.contains("gateway.ai.cloudflare.com");
    let is_nvidia = provider == "nvidia" || base_url.contains("integrate.api.nvidia.com");
    let is_ant_ling = provider == "ant-ling" || base_url.contains("api.ant-ling.com");
    let is_grok = provider == "xai" || base_url.contains("api.x.ai");
    let is_deepseek = provider == "deepseek" || base_url.contains("deepseek.com");
    let is_mistral = provider == "mistral" || base_url.contains("api.mistral.ai");
    
    let is_anthropic_native = provider == "anthropic";

    let is_nonstandard = is_nvidia
        || provider == "cerebras"
        || base_url.contains("cerebras.ai")
        || is_grok
        || is_together
        || base_url.contains("chutes.ai")
        || is_deepseek
        || is_zai
        || is_moonshot
        || provider == "opencode"
        || base_url.contains("opencode.ai")
        || is_cloudflare_workers
        || is_cloudflare_gateway
        || is_ant_ling;

    let uses_max_tokens = base_url.contains("chutes.ai")
        || is_moonshot
        || is_cloudflare_gateway
        || is_together
        || is_nvidia
        || is_ant_ling;

    
    let openrouter_developer_role =
        is_openrouter && (model_id.starts_with("anthropic/") || model_id.starts_with("openai/"));

    Compat {
        supports_store: !is_nonstandard,
        supports_developer_role: openrouter_developer_role || (!is_nonstandard && !is_openrouter),
        supports_reasoning_effort: !is_grok
            && !is_zai
            && !is_moonshot
            && !is_together
            && !is_cloudflare_gateway
            && !is_nvidia
            && !is_ant_ling,
        supports_usage_in_streaming: true,
        max_tokens_field: match uses_max_tokens {
            true => MaxTokensField::MaxTokens,
            false => MaxTokensField::MaxCompletionTokens,
        },
        requires_reasoning_content_on_assistant_messages: is_deepseek,
        thinking_format: if is_deepseek {
            ThinkingFormat::Deepseek
        } else if is_zai {
            ThinkingFormat::Zai
        } else if is_together {
            ThinkingFormat::Together
        } else if is_ant_ling {
            ThinkingFormat::AntLing
        } else if is_openrouter {
            ThinkingFormat::Openrouter
        } else {
            ThinkingFormat::Openai
        },
        zai_tool_stream: false,
        supports_strict_mode: !is_moonshot && !is_together && !is_cloudflare_gateway && !is_nvidia,
        supports_strict_tools: is_anthropic_native,
        
        bedrock_supports_strict_tools: false,
        cache_control_format: match provider == "openrouter" && model_id.starts_with("anthropic/") {
            true => Some(CacheControlFormat::Anthropic),
            false => None,
        },
        send_session_affinity_headers: is_mistral,
        session_affinity_format: if is_mistral {
            SessionAffinity::Mistral
        } else if is_openrouter {
            SessionAffinity::Openrouter
        } else {
            SessionAffinity::Openai
        },
        supports_long_cache_retention: !(is_together
            || is_cloudflare_workers
            || is_cloudflare_gateway
            || is_nvidia
            || is_ant_ling),
        supports_temperature: true,
        supports_eager_tool_input_streaming: true,
        supports_cache_control_on_tools: true,
        force_adaptive_thinking: decides_its_own_thinking(model_id),
        
        tool_call_id_length: is_mistral.then_some(MISTRAL_TOOL_CALL_ID_LENGTH),
        thinking: BTreeMap::new(),
    }
}

/// Whether a model is asked for an effort and left to decide how much to think.
const MISTRAL_TOOL_CALL_ID_LENGTH: usize = 9;

fn decides_its_own_thinking(model_id: &str) -> bool {
    const ADAPTIVE: &[&str] = &[
        "opus-4-6",
        "opus-4.6",
        "opus-4-7",
        "opus-4.7",
        "opus-4-8",
        "opus-4.8",
        "opus-5",
        "opus.5",
        "sonnet-4-6",
        "sonnet-4.6",
        "sonnet-5",
        "sonnet.5",
        "fable-5",
    ];
    ADAPTIVE.iter().any(|name| model_id.contains(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_itself_takes_the_protocol_as_written() {
        let compat = detect("openai", "https://api.openai.com/v1", "gpt-5.6-terra");
        assert!(compat.supports_store);
        assert!(compat.supports_developer_role);
        assert_eq!(compat.max_tokens_field, MaxTokensField::MaxCompletionTokens);
        assert_eq!(compat.thinking_format, ThinkingFormat::Openai);
    }

    /// Only Anthropic's own service is assumed to have kept pace with a beta feature.
    #[test]
    fn only_anthropics_own_service_is_assumed_to_support_strict_tools() {
        let anthropic = detect("anthropic", "https://api.anthropic.com/v1", "claude-opus-5");
        assert!(anthropic.supports_strict_tools);

        let copilot = detect(
            "github-copilot",
            "https://api.githubcopilot.com",
            "claude-fable-5",
        );
        assert!(!copilot.supports_strict_tools);

        let openrouter = detect(
            "openrouter",
            "https://openrouter.ai/api/v1",
            "anthropic/claude-opus-5",
        );
        assert!(!openrouter.supports_strict_tools);
    }

    
    #[test]
    fn nothing_infers_bedrock_strict_tool_support() {
        let bedrock = detect(
            "bedrock",
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            "anthropic.claude-opus-4-v1:0",
        );
        assert!(!bedrock.bedrock_supports_strict_tools);
    }

    /// A service that reimplements the protocol rarely takes all of it.
    #[test]
    fn a_reimplementation_is_assumed_to_take_less() {
        let compat = detect("cerebras", "https://api.cerebras.ai/v1", "gpt-oss-120b");
        assert!(!compat.supports_store);
        assert!(!compat.supports_developer_role);

        let together = detect(
            "together",
            "https://api.together.ai/v1",
            "openai/gpt-oss-20b",
        );
        assert_eq!(together.max_tokens_field, MaxTokensField::MaxTokens);
        assert_eq!(together.thinking_format, ThinkingFormat::Together);
        assert!(!together.supports_strict_mode);
        assert!(!together.supports_long_cache_retention);
    }

    /// A gateway's answer follows whoever is behind it.
    #[test]
    fn a_gateway_follows_the_model_it_is_passing_through_to() {
        let anthropic = detect(
            "openrouter",
            "https://openrouter.ai/api/v1",
            "anthropic/claude-opus-5",
        );
        assert!(anthropic.supports_developer_role);
        assert_eq!(
            anthropic.cache_control_format,
            Some(CacheControlFormat::Anthropic)
        );
        assert_eq!(anthropic.thinking_format, ThinkingFormat::Openrouter);
        assert_eq!(
            anthropic.session_affinity_format,
            SessionAffinity::Openrouter
        );

        let other = detect(
            "openrouter",
            "https://openrouter.ai/api/v1",
            "qwen/qwen3-max",
        );
        assert!(!other.supports_developer_role);
        assert_eq!(other.cache_control_format, None);
    }

    #[test]
    fn what_an_entry_states_wins_over_what_was_inferred() {
        let stated = CompatOverrides {
            supports_store: Some(true),
            thinking_format: Some(ThinkingFormat::StringThinking),
            ..CompatOverrides::default()
        };
        let compat = resolve(
            "cerebras",
            "https://api.cerebras.ai/v1",
            "gpt-oss-120b",
            &stated,
            &BTreeMap::new(),
        );

        assert!(compat.supports_store);
        assert_eq!(compat.thinking_format, ThinkingFormat::StringThinking);
        
        assert!(!compat.supports_developer_role);
    }

    #[test]
    fn a_level_a_model_does_not_offer_is_carried_as_such() {
        let thinking = BTreeMap::from([
            ("off".to_string(), None),
            ("low".to_string(), Some("high".to_string())),
        ]);
        let compat = resolve(
            "zai",
            "https://api.z.ai/api/coding/paas/v4",
            "glm-5",
            &Default::default(),
            &thinking,
        );

        assert_eq!(compat.level(micro_types::ThinkingLevel::Off), None);
        assert_eq!(
            compat.level(micro_types::ThinkingLevel::Low),
            Some("high".to_string())
        );
        
        assert_eq!(
            compat.level(micro_types::ThinkingLevel::High),
            Some("high".to_string())
        );
    }

    /// Which Claude models decide their own thinking is read from the id, both spellings.
    #[test]
    fn the_newest_claude_models_decide_their_own_thinking() {
        let adaptive = [
            "claude-opus-4-6",
            "claude-opus-4.6-20260101",
            "claude-opus-4-7",
            "claude-opus-4-8",
            "claude-opus-5",
            "claude-sonnet-4-6",
            "claude-sonnet-5",
            "claude-fable-5",
            "anthropic/claude-opus-5",
        ];
        for id in adaptive {
            assert!(decides_its_own_thinking(id), "{id} is asked for an effort",);
        }

        let budgeted = [
            "claude-opus-4-1",
            "claude-3-5-sonnet-20241022",
            "claude-sonnet-4-5",
            "claude-haiku-4-5",
        ];
        for id in budgeted {
            assert!(!decides_its_own_thinking(id), "{id} is given a budget");
        }
    }

    /// The flag reaches the resolved compat, and the catalog can still say otherwise.
    #[test]
    fn the_catalog_can_override_how_a_model_is_asked_to_think() {
        let compat = resolve(
            "anthropic",
            "https://api.anthropic.com/v1",
            "claude-opus-5",
            &Default::default(),
            &BTreeMap::new(),
        );
        assert!(compat.force_adaptive_thinking);

        let stated = CompatOverrides {
            force_adaptive_thinking: Some(false),
            ..Default::default()
        };
        let compat = resolve(
            "anthropic",
            "https://api.anthropic.com/v1",
            "claude-opus-5",
            &stated,
            &BTreeMap::new(),
        );
        assert!(!compat.force_adaptive_thinking);
    }
}

#[cfg(test)]
mod catalog_probe {
    /// Every override the catalog states must survive being read.
    #[test]
    fn what_the_catalog_states_about_a_model_is_read() {
        let catalog = crate::Catalog::bundled();
        let model = catalog
            .get("github-copilot", "claude-fable-5")
            .expect("the bundled catalog lists this model");
        let compat = model.to_runtime(micro_types::ThinkingLevel::Off).compat;
        assert!(!compat.supports_store, "the catalog says it does not store");
        assert!(!compat.supports_developer_role);
        assert!(!compat.supports_reasoning_effort);
    }

    /// No key the catalog states may be dropped on the way in.
    #[test]
    fn no_compat_key_in_the_catalog_is_silently_dropped() {
        let file: serde_json::Value =
            serde_json::from_str(crate::bundled::CATALOG_JSON).expect("the catalog parses");

        let mut unread: std::collections::BTreeSet<String> = Default::default();
        let providers = file["providers"].as_object().expect("providers");
        for provider in providers.values() {
            let Some(models) = provider["models"].as_array() else {
                continue;
            };
            for model in models {
                let Some(stated) = model.get("compat").and_then(|compat| compat.as_object()) else {
                    continue;
                };
                let parsed: super::CompatOverrides =
                    serde_json::from_value(serde_json::Value::Object(stated.clone()))
                        .expect("a compat block parses");
                let read_back = serde_json::to_value(&parsed).expect("and serializes");
                for key in stated.keys() {
                    if read_back.get(key).is_none() {
                        unread.insert(key.clone());
                    }
                }
            }
        }

        assert!(
            unread.is_empty(),
            "the catalog states these and nothing reads them: {unread:?}",
        );
    }
}

#[cfg(test)]
mod mistral {
    use super::*;

    
    #[test]
    fn mistral_is_spoken_to_the_way_it_expects() {
        let catalog = crate::Catalog::bundled();
        let model = catalog
            .models()
            .iter()
            .find(|model| model.provider == "mistral")
            .expect("the bundled catalog serves Mistral");

        let compat = model.to_runtime(micro_types::ThinkingLevel::Off).compat;
        assert!(compat.send_session_affinity_headers);
        assert_eq!(
            compat.session_affinity_format,
            micro_types::SessionAffinity::Mistral
        );
        assert_eq!(compat.tool_call_id_length, Some(9));
    }

    /// Everyone else is left alone: an id limit is Mistral's rule, not the protocol's.
    #[test]
    fn nobody_else_is_given_mistrals_rules() {
        let compat = resolve(
            "openai",
            "https://api.openai.com/v1",
            "gpt-5.5",
            &Default::default(),
            &BTreeMap::new(),
        );
        assert_eq!(compat.tool_call_id_length, None);
        assert!(!compat.send_session_affinity_headers);
    }
}
