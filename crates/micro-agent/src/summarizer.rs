//! Summarizing a conversation by asking the model that produced it.

use async_trait::async_trait;
use micro_context::ContextError;
use micro_context::Result;
use micro_context::Summarizer;
use micro_context::COMPACTION_PROMPT;
use micro_provider::ApiKey;
use micro_provider::Provider;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::StreamEvent;
use micro_types::ThinkingLevel;
use std::sync::Arc;

/// A [`Summarizer`] backed by the same provider the agent talks to.
///
/// The request carries no tools and one user message, so the model has nothing to do but
/// write the summary.
pub struct ProviderSummarizer {
    provider: Arc<dyn Provider>,
    model: Model,
    api_key: ApiKey,
}

impl ProviderSummarizer {
    pub fn new(provider: Arc<dyn Provider>, model: Model, api_key: impl Into<ApiKey>) -> Self {
        ProviderSummarizer {
            provider,
            model,
            api_key: api_key.into(),
        }
    }
}

#[async_trait]
impl Summarizer for ProviderSummarizer {
    async fn summarize(&self, messages: &[Message]) -> Result<String> {
        let transcript = micro_context::render_transcript(messages);
        let context = Context {
            system_prompt: None,
            messages: vec![Message::user(format!("{transcript}\n{COMPACTION_PROMPT}"))],
            tools: Vec::new(),
            headers: Vec::new(),
            cache_key: None,
        };

        let mut model = self.model.clone();
        // Restating a transcript is not a reasoning task, and thinking output is discarded
        // rather than kept as the summary.
        model.thinking = ThinkingLevel::Off;

        let mut stream = self
            .provider
            .stream(model, context, self.api_key.current().await);

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Done { message } => {
                    let summary = message.text().trim().to_string();
                    // An empty summary would replace the history with nothing, which loses
                    // the conversation more thoroughly than not compacting at all.
                    if summary.is_empty() {
                        return Err(ContextError::summarizer("the model returned no summary"));
                    }
                    return Ok(summary);
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
