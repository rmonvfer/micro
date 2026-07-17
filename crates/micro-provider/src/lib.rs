//! Provider clients. A provider turns a [`Context`] into an HTTP request and the
//! response body into a stream of [`StreamEvent`]s.
//!
//! [`registry`] is the way in for a caller that picks a provider at runtime.

mod anthropic;
mod gemini;
mod json;
mod openai;
mod registry;
mod sse;

pub use anthropic::Anthropic;
pub use gemini::Gemini;
pub use openai::OpenAi;
pub use registry::known_providers;
pub use registry::model_for;
pub use registry::provider_by_name;
pub use registry::provider_info;
pub use registry::resolve;
pub use registry::ProviderInfo;
pub use registry::ResolveError;
pub use registry::ResolvedProvider;
pub use sse::SseEvent;

/// Re-exported so a caller that renders a provider picker needs only this crate.
pub use micro_auth::AuthMethod;

use micro_types::Context;
use micro_types::Model;
use micro_types::StreamEvent;
use tokio::sync::mpsc::UnboundedReceiver;

/// Starts streaming requests against one wire format.
///
/// `stream` returns immediately; the request runs on a spawned task and pushes events
/// into the returned receiver. The final event is always [`StreamEvent::Done`] or
/// [`StreamEvent::Error`], so a consumer can drain until one of those arrives.
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;

    fn stream(
        &self,
        model: Model,
        context: Context,
        api_key: String,
    ) -> UnboundedReceiver<StreamEvent>;
}

/// How long a request may go without producing anything before it is given up on.
///
/// A model that is thinking sends nothing for a while, so this is generous: it is there to
/// notice a connection that has died, not to hurry an answer along.
static IDLE_TIMEOUT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(120);

/// Set the idle timeout every provider built from here on will use.
pub fn set_idle_timeout(seconds: u64) {
    IDLE_TIMEOUT.store(seconds.max(1), std::sync::atomic::Ordering::Relaxed);
}

pub fn idle_timeout() -> std::time::Duration {
    std::time::Duration::from_secs(IDLE_TIMEOUT.load(std::sync::atomic::Ordering::Relaxed))
}

/// An HTTP client that gives up on a connection that has gone quiet.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .read_timeout(idle_timeout())
        .build()
        .unwrap_or_default()
}
