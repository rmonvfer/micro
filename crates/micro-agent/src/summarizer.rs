//! Summarizing a conversation by asking the model that produced it.

use async_trait::async_trait;
use micro_context::ContextError;
use micro_context::Result;
use micro_context::Summarizer;
use micro_context::Summary;
use micro_context::COMPACTION_PROMPT;
use micro_provider::ApiKey;
use micro_provider::Provider;
use micro_types::CompactionCost;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::ModelPricing;
use micro_types::StreamEvent;
use micro_types::ThinkingLevel;
use std::sync::Arc;

/// A [`Summarizer`] backed by the same provider the agent talks to.
pub struct ProviderSummarizer {
    provider: Arc<dyn Provider>,
    model: Model,
    api_key: ApiKey,
    pricing: Option<ModelPricing>,
    /// What the conversation being summarized is called.
    cache_key: Option<String>,
}

impl ProviderSummarizer {
    pub fn new(provider: Arc<dyn Provider>, model: Model, api_key: impl Into<ApiKey>) -> Self {
        ProviderSummarizer {
            provider,
            model,
            api_key: api_key.into(),
            pricing: None,
            cache_key: None,
        }
    }

    /// Record the rates used to price the summary this model writes.
    pub fn with_pricing(mut self, pricing: ModelPricing) -> Self {
        self.pricing = Some(pricing);
        self
    }

    /// Summarize as part of a named conversation.
    pub fn for_conversation(mut self, key: Option<String>) -> Self {
        self.cache_key = key;
        self
    }
}

#[async_trait]
impl Summarizer for ProviderSummarizer {
    async fn summarize(&self, messages: &[Message]) -> Result<Summary> {
        let transcript = micro_context::render_transcript(messages);
        let context = Context {
            system_prompt: None,
            messages: vec![Message::user(format!("{transcript}\n{COMPACTION_PROMPT}"))],
            tools: Vec::new(),
            headers: Vec::new(),
            cache_key: self.cache_key.clone(),
        };

        let mut model = self.model.clone();

        model.thinking = ThinkingLevel::Off;

        let api_key = self
            .api_key
            .current()
            .await
            .map_err(ContextError::summarizer)?;
        let mut stream = self.provider.stream(model, context, api_key);

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Done { message } => {
                    let summary = message.text().trim().to_string();

                    if summary.is_empty() {
                        return Err(ContextError::summarizer("the model returned no summary"));
                    }
                    return Ok(Summary {
                        text: summary,

                        cost: CompactionCost {
                            usage: message.usage,
                            provider: message.provider,
                            model: message.model,
                            pricing: self.pricing.clone(),
                        },
                    });
                }
                StreamEvent::Error { message } => return Err(ContextError::summarizer(message)),
                _ => {}
            }
        }

        Err(ContextError::summarizer(
            "the provider closed the stream without a summary",
        ))
    }
}
