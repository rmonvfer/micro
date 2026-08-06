//! What a service accepts, worked out from who serves a model and where.
//!
//! A catalog entry records only what a service does differently from the protocol it
//! answers. Everything else is inferred from the provider and its address, the way ohm
//! infers it in `packages/ai/src/api/openai-completions.ts`: the inference is the base,
//! and whatever the entry states is laid over it.

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
    let is_moonshot =
        provider == "moonshotai" || provider == "moonshotai-cn" || base_url.contains("api.moonshot.");
    let is_openrouter = provider == "openrouter" || base_url.contains("openrouter.ai");
    let is_cloudflare_workers =
        provider == "cloudflare-workers-ai" || base_url.contains("api.cloudflare.com");
    let is_cloudflare_gateway =
        provider == "cloudflare-ai-gateway" || base_url.contains("gateway.ai.cloudflare.com");
    let is_nvidia = provider == "nvidia" || base_url.contains("integrate.api.nvidia.com");
    let is_ant_ling = provider == "ant-ling" || base_url.contains("api.ant-ling.com");
    let is_grok = provider == "xai" || base_url.contains("api.x.ai");
    let is_deepseek = provider == "deepseek" || base_url.contains("deepseek.com");

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

    // OpenRouter passes a request through to whoever serves the model, so what it accepts
    // follows the model rather than the gateway.
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
        cache_control_format: match provider == "openrouter" && model_id.starts_with("anthropic/") {
            true => Some(CacheControlFormat::Anthropic),
            false => None,
        },
        send_session_affinity_headers: false,
        session_affinity_format: match is_openrouter {
            true => SessionAffinity::Openrouter,
            false => SessionAffinity::Openai,
        },
        supports_long_cache_retention: !(is_together
            || is_cloudflare_workers
            || is_cloudflare_gateway
            || is_nvidia
            || is_ant_ling),
        supports_temperature: true,
        supports_eager_tool_input_streaming: true,
        supports_cache_control_on_tools: true,
        thinking: BTreeMap::new(),
    }
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

    /// A service that reimplements the protocol rarely takes all of it.
    #[test]
    fn a_reimplementation_is_assumed_to_take_less() {
        let compat = detect("cerebras", "https://api.cerebras.ai/v1", "gpt-oss-120b");
        assert!(!compat.supports_store);
        assert!(!compat.supports_developer_role);

        let together = detect("together", "https://api.together.ai/v1", "openai/gpt-oss-20b");
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
        assert_eq!(anthropic.session_affinity_format, SessionAffinity::Openrouter);

        let other = detect("openrouter", "https://openrouter.ai/api/v1", "qwen/qwen3-max");
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
        // Anything it does not state is still inferred.
        assert!(!compat.supports_developer_role);
    }

    #[test]
    fn a_level_a_model_does_not_offer_is_carried_as_such() {
        let thinking = BTreeMap::from([
            ("off".to_string(), None),
            ("low".to_string(), Some("high".to_string())),
        ]);
        let compat = resolve("zai", "https://api.z.ai/api/coding/paas/v4", "glm-5", &Default::default(), &thinking);

        assert_eq!(compat.level(micro_types::ThinkingLevel::Off), None);
        assert_eq!(
            compat.level(micro_types::ThinkingLevel::Low),
            Some("high".to_string())
        );
        // A level it says nothing about keeps the name the protocol gives it.
        assert_eq!(
            compat.level(micro_types::ThinkingLevel::High),
            Some("high".to_string())
        );
    }
}
