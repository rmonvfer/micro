//! A [`Summarizer`] whose output a test decides, and which records what it was asked to
//! summarize.

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use micro_context::ContextError;
use micro_context::Result;
use micro_context::Summarizer;
use micro_context::Summary;
use micro_types::Message;

/// A summarizer that answers with canned text and records every conversation handed to it.
///
/// Using this instead of the agent's own provider-backed summarizer keeps a compaction
/// test's scripted turns for the turns under test, rather than spending one on the summary.
#[derive(Clone)]
pub struct FakeSummarizer {
    inner: Arc<Inner>,
}

struct Inner {
    answer: std::result::Result<String, String>,
    calls: Mutex<Vec<Vec<Message>>>,
}

impl FakeSummarizer {
    /// A summarizer that always produces `summary`.
    pub fn new(summary: impl Into<String>) -> Self {
        FakeSummarizer::answering(Ok(summary.into()))
    }

    /// A summarizer that always fails, for testing that a run survives it.
    pub fn failing(reason: impl Into<String>) -> Self {
        FakeSummarizer::answering(Err(reason.into()))
    }

    fn answering(answer: std::result::Result<String, String>) -> Self {
        FakeSummarizer {
            inner: Arc::new(Inner {
                answer,
                calls: Mutex::new(Vec::new()),
            }),
        }
    }

    /// The conversations it was asked to summarize, oldest first.
    pub fn calls(&self) -> Vec<Vec<Message>> {
        self.inner.calls.lock().expect("calls lock").clone()
    }

    pub fn call_count(&self) -> usize {
        self.inner.calls.lock().expect("calls lock").len()
    }

    /// The messages summarized on call `index`, panicking with a legible message when it
    /// was called fewer times than that.
    pub fn call(&self, index: usize) -> Vec<Message> {
        let calls = self.calls();
        assert!(
            index < calls.len(),
            "expected the summarizer to run at least {} time(s), it ran {}",
            index + 1,
            calls.len()
        );
        calls[index].clone()
    }
}

#[async_trait]
impl Summarizer for FakeSummarizer {
    async fn summarize(&self, messages: &[Message]) -> Result<Summary> {
        self.inner
            .calls
            .lock()
            .expect("calls lock")
            .push(messages.to_vec());

        self.inner
            .answer
            .clone()
            .map(Summary::text)
            .map_err(ContextError::summarizer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_canned_summary_is_returned_and_the_input_recorded() {
        let summarizer = FakeSummarizer::new("what happened");
        let messages = vec![Message::user("hello"), Message::user("again")];

        let summary = summarizer.summarize(&messages).await.unwrap();

        assert_eq!(summary.text, "what happened");
        assert_eq!(summarizer.call_count(), 1);
        assert_eq!(summarizer.call(0), messages);
    }

    #[tokio::test]
    async fn a_failing_summarizer_reports_its_reason() {
        let summarizer = FakeSummarizer::failing("the model said no");

        let error = summarizer.summarize(&[]).await.unwrap_err();

        assert!(error.to_string().contains("the model said no"));
        assert_eq!(summarizer.call_count(), 1);
    }

    #[tokio::test]
    async fn clones_share_one_call_log() {
        let summarizer = FakeSummarizer::new("s");
        let clone = summarizer.clone();

        clone.summarize(&[Message::user("hi")]).await.unwrap();

        assert_eq!(summarizer.call_count(), 1);
    }
}
