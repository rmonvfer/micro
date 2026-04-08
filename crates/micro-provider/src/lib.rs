//! Provider clients. A provider turns a [`Context`] into an HTTP request and the
//! response body into a stream of [`StreamEvent`]s.
//!
//! [`registry`] is the way in for a caller that picks a provider at runtime.

mod anthropic;
mod bedrock;
mod codex;
mod constrained_sampling;
mod credential;
mod eventstream;
mod gemini;
mod json;
mod openai;
mod registry;
mod sigv4;
mod sse;
mod vertex;

pub use anthropic::Anthropic;
pub use codex::Codex;
pub use codex::Transport;
pub use credential::ApiKey;
pub use gemini::Gemini;
pub use openai::OpenAi;
pub use registry::client_for;
pub use registry::client_for_model;
pub use registry::known_providers;
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

    /// The body [`Provider::stream`] would send for this model and this context.
    ///
    /// The same assembly the request itself goes through, so what comes back is what the
    /// service is told rather than a description of it. That is what lets a session record
    /// a request by its hash and rebuild it afterwards without storing a copy of every
    /// body it ever sent.
    ///
    /// A body that cannot be assembled — a tool schema the service would refuse — is
    /// answered as null rather than as an error: this is a reading of the request, and the
    /// attempt to send it is where the refusal belongs.
    fn payload(&self, model: &Model, context: &Context) -> serde_json::Value;
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

/// What micro tells a service about itself.
///
/// Not telemetry: nothing is measured, nothing is sent anywhere micro chose, and there is
/// no setting to turn off because there is nothing being collected. These are the headers a
/// few services read to know which client is asking — OpenRouter lists the harnesses using
/// it, and Nvidia and Cloudflare attribute a call the same way. A request carries the name
/// of the program that made it, and that is all this is.
pub fn attribution(base_url: &str) -> &'static [(&'static str, &'static str)] {
    const OPENROUTER: &[(&str, &str)] = &[
        ("HTTP-Referer", "https://github.com/rmonvfer/micro"),
        ("X-OpenRouter-Title", "micro"),
        ("X-OpenRouter-Categories", "cli-agent"),
    ];
    const NVIDIA: &[(&str, &str)] = &[("X-BILLING-INVOKE-ORIGIN", "micro")];
    const CLOUDFLARE: &[(&str, &str)] = &[("User-Agent", "micro")];

    match host_of(base_url).as_deref() {
        Some("openrouter.ai") => OPENROUTER,
        Some("integrate.api.nvidia.com") => NVIDIA,
        Some("api.cloudflare.com" | "gateway.ai.cloudflare.com") => CLOUDFLARE,
        _ => &[],
    }
}

/// The host an address names, for deciding what a service is without matching on the whole
/// of its URL.
fn host_of(base_url: &str) -> Option<String> {
    let rest = base_url.split_once("://").map(|(_, rest)| rest)?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(host, _)| host);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Say which program is asking, where the service reads it.
pub(crate) fn with_attribution(
    mut request: reqwest::RequestBuilder,
    base_url: &str,
) -> reqwest::RequestBuilder {
    for (name, value) in attribution(base_url) {
        request = request.header(*name, *value);
    }
    request
}

/// Say which program is asking, then put the caller's own headers on top.
///
/// The caller's are applied last, so a header named by whoever assembled the request
/// replaces both the one the provider set for itself and the one naming micro.
pub(crate) fn with_carried_headers(
    request: reqwest::RequestBuilder,
    context: &micro_types::Context,
    base_url: &str,
) -> reqwest::RequestBuilder {
    let mut request = with_attribution(request, base_url);
    for (name, value) in &context.headers {
        request = request.header(name.as_str(), value.as_str());
    }
    request
}

#[cfg(test)]
mod attribution_tests {
    use super::*;

    /// A service that reads which client is asking is told micro, and one that does not is
    /// told nothing. No setting gates this because nothing is being collected: it is the
    /// name of the program on its own request.
    #[test]
    fn a_request_says_which_program_made_it() {
        let named =
            |url: &str| -> Vec<&str> { attribution(url).iter().map(|(name, _)| *name).collect() };

        assert_eq!(
            named("https://openrouter.ai/api/v1"),
            vec![
                "HTTP-Referer",
                "X-OpenRouter-Title",
                "X-OpenRouter-Categories"
            ]
        );
        assert_eq!(
            attribution("https://openrouter.ai/api/v1")[1].1,
            "micro",
            "listed under its own name"
        );
        assert_eq!(
            named("https://integrate.api.nvidia.com/v1"),
            vec!["X-BILLING-INVOKE-ORIGIN"]
        );
        assert_eq!(
            named("https://gateway.ai.cloudflare.com/v1/x/y"),
            vec!["User-Agent"]
        );
        assert!(named("https://api.anthropic.com/v1").is_empty());
        assert!(named("https://api.openai.com/v1").is_empty());
    }

    /// The host decides, so a port, a userinfo or a path cannot make one service look like
    /// another — and something that is not an address at all names nobody.
    #[test]
    fn the_host_is_read_from_the_address_and_nothing_else() {
        assert_eq!(
            host_of("https://openrouter.ai:443/api").as_deref(),
            Some("openrouter.ai")
        );
        assert_eq!(
            host_of("https://user@OpenRouter.AI/api").as_deref(),
            Some("openrouter.ai")
        );
        assert_eq!(host_of("not a url").as_deref(), None);
        assert!(attribution("https://example.test/openrouter.ai").is_empty());
    }
}
