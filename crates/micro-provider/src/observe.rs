//! Parsed provider events, handed to whoever is watching before micro reads them into its own
//! stream events.
//!
//! What a watcher receives is the earliest structured value micro has of each event: an SSE
//! frame's JSON, or a Bedrock event's payload. It is not the original bytes. Watching costs
//! nothing while nobody watches.

use serde_json::Value;
use std::sync::RwLock;
use tokio::sync::mpsc::UnboundedSender;

/// One event a provider sent, as parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderEvent {
    pub provider: String,
    /// The protocol, by its pi-ai name.
    pub api: &'static str,
    pub model: String,
    pub data: Value,
}

static WATCHER: RwLock<Option<UnboundedSender<ProviderEvent>>> = RwLock::new(None);

/// Send every parsed provider event from here on to `watcher`, or to nobody.
pub fn watch_provider_events(watcher: Option<UnboundedSender<ProviderEvent>>) {
    if let Ok(mut held) = WATCHER.write() {
        *held = watcher;
    }
}

/// Hand one parsed event to the watcher, in the order it arrived.
pub(crate) fn parsed(provider: &str, api: &'static str, model: &str, data: &Value) {
    let Ok(held) = WATCHER.read() else {
        return;
    };
    let Some(watcher) = held.as_ref() else {
        return;
    };
    let _ = watcher.send(ProviderEvent {
        provider: provider.to_string(),
        api,
        model: model.to_string(),
        data: data.clone(),
    });
}
