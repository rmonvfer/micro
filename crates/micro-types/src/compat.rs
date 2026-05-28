//! What one service needs on top of the protocol it speaks.

use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;

/// Which field carries the output limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaxTokensField {
    #[default]
    MaxCompletionTokens,
    MaxTokens,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkingFormat {
    /// A top-level `reasoning_effort`.
    #[default]
    Openai,
    /// A nested `reasoning: { effort }`, which is how OpenRouter normalizes it.
    Openrouter,
    /// A `thinking: { type }` alongside `reasoning_effort`.
    Deepseek,
    /// A `reasoning: { enabled }` alongside `reasoning_effort`.
    Together,
    /// A `thinking: { type, clear_thinking }`.
    Zai,
    /// A nested `reasoning: { effort }`, sent only for a level the model maps.
    AntLing,
    /// A top-level `enable_thinking`.
    Qwen,
    /// `chat_template_kwargs`, carrying the thinking switch.
    QwenChatTemplate,
    /// `chat_template_kwargs`, configured by the model.
    ChatTemplate,
    /// A top-level `thinking` naming the level.
    StringThinking,
}

/// Which headers tie a request to the conversation it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionAffinity {
    /// `session_id`, `x-client-request-id` and `x-session-affinity`.
    #[default]
    Openai,
    /// The same, without the session id.
    OpenaiNosession,
    /// `x-session-id`.
    Openrouter,
    /// `x-affinity`, which is how Mistral routes a conversation back to the machine holding its
    /// cached prompt.
    Mistral,
}

/// How a prompt is marked for caching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheControlFormat {
    /// Anthropic-style `cache_control` markers, which some compatible services accept.
    Anthropic,
}

/// Everything decided about how one service is spoken to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Compat {
    /// Whether the service understands `store`.
    pub supports_store: bool,
    /// Whether it takes a `developer` role in place of `system`.
    pub supports_developer_role: bool,
    /// Whether it takes a reasoning effort at all.
    pub supports_reasoning_effort: bool,
    /// Whether a streaming response can carry a usage report.
    pub supports_usage_in_streaming: bool,
    pub max_tokens_field: MaxTokensField,
    /// Whether replayed assistant messages need an empty reasoning field.
    pub requires_reasoning_content_on_assistant_messages: bool,
    pub thinking_format: ThinkingFormat,
    /// Whether it takes `tool_stream`, which streams tool arguments as they arrive.
    pub zai_tool_stream: bool,
    /// Whether a tool definition may carry `strict`.
    pub supports_strict_mode: bool,
    /// Whether Anthropic's Messages API will honor `strict` on a tool definition.
    pub supports_strict_tools: bool,
    /// Whether Bedrock's Converse API will honor `strict` on a tool spec.
    pub bedrock_supports_strict_tools: bool,
    pub cache_control_format: Option<CacheControlFormat>,
    /// Whether to tie a request to its conversation with headers.
    pub send_session_affinity_headers: bool,
    pub session_affinity_format: SessionAffinity,
    /// Whether a prompt may be cached for longer than the default few minutes.
    pub supports_long_cache_retention: bool,
    /// Whether a temperature may be sent.
    pub supports_temperature: bool,
    /// Whether tool definitions may ask for their input to stream eagerly.
    pub supports_eager_tool_input_streaming: bool,
    /// Whether a tool definition may carry a cache marker.
    pub supports_cache_control_on_tools: bool,
    /// How long a tool call's id may be, when the service will not take an arbitrary one.
    pub tool_call_id_length: Option<usize>,
    
    pub force_adaptive_thinking: bool,
    /// What each thinking level is called here.
    pub thinking: BTreeMap<String, Option<String>>,
}

impl Default for Compat {
    /// What a service is assumed to accept when it says nothing: the protocol as its originator
    /// defines it.
    fn default() -> Self {
        Compat {
            supports_store: true,
            supports_developer_role: true,
            supports_reasoning_effort: true,
            supports_usage_in_streaming: true,
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            requires_reasoning_content_on_assistant_messages: false,
            thinking_format: ThinkingFormat::Openai,
            zai_tool_stream: false,
            supports_strict_mode: true,
            supports_strict_tools: false,
            bedrock_supports_strict_tools: false,
            cache_control_format: None,
            send_session_affinity_headers: false,
            session_affinity_format: SessionAffinity::Openai,
            supports_long_cache_retention: true,
            supports_temperature: true,
            supports_eager_tool_input_streaming: true,
            supports_cache_control_on_tools: true,
            tool_call_id_length: None,
            force_adaptive_thinking: false,
            thinking: BTreeMap::new(),
        }
    }
}

impl Compat {
    /// What this service calls one thinking level, or nothing when it does not offer it.
    pub fn level(&self, level: crate::ThinkingLevel) -> Option<String> {
        let name = level.as_str();
        match self.thinking.get(name) {
            Some(mapped) => mapped.clone(),
            None => Some(name.to_string()),
        }
    }

    /// What this model calls thinking being off, as far as it says anything.
    pub fn off(&self) -> OffLevel {
        match self.thinking.get("off") {
            Some(Some(named)) => OffLevel::Named(named.clone()),
            Some(None) => OffLevel::Unsupported,
            None => OffLevel::Unsaid,
        }
    }
}

/// What a model says about thinking being off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OffLevel {
    /// The service takes this name for it.
    Named(String),
    /// The model cannot be asked to stop thinking, so nothing is sent.
    Unsupported,
    /// The model says nothing, and each protocol has its own default.
    Unsaid,
}

impl OffLevel {
    /// The name to send where a protocol has one of its own to fall back on.
    pub fn or(&self, fallback: &str) -> Option<String> {
        match self {
            OffLevel::Named(named) => Some(named.clone()),
            OffLevel::Unsaid => Some(fallback.to_string()),
            OffLevel::Unsupported => None,
        }
    }
}
