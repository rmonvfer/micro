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
