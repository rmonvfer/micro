//! Virtual models: models an extension registers whose router picks a physical model and a
//! thinking level for every request.
//!
//! The selection stays the virtual model; each request is dispatched to a physical one, whose
//! name the assistant message carries, so the transcript, the bill and the footer all say which
//! model answered.

use micro_extensions::Host;
use micro_extensions::RegisteredVirtualModel;
use micro_models::Catalog;
use micro_models::Modality;
use micro_models::ModelDef;
use micro_models::ModelType;
use micro_models::WireApi;
use micro_provider::ModelRuntime;
use micro_provider::Provider;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::StreamEvent;
use micro_types::ThinkingLevel;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

/// The custom entry router state is kept in, on the session branch it belongs to.
pub(crate) const STATE_ENTRY: &str = "virtual-model-state";

/// What a virtual model is assumed to hold when its extension does not say, for compaction to
/// work from before the first response.
const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;
const DEFAULT_MAX_TOKENS: u32 = 16_384;

/// A registered virtual model, as the catalog lists it.
pub(crate) fn catalog_entry(registered: &RegisteredVirtualModel) -> ModelDef {
    let input = registered
        .input
        .iter()
        .filter_map(|name| match name.as_str() {
            "text" => Some(Modality::Text),
            "image" => Some(Modality::Image),
            _ => None,
        })
        .collect();
    let thinking = ThinkingLevel::ALL
        .into_iter()
        .map(|level| {
            let offered = registered
                .thinking_levels
                .iter()
                .any(|name| name == level.as_str());
            (
                level.as_str().to_string(),
                offered.then(|| level.as_str().to_string()),
            )
        })
        .collect();
    ModelDef {
        id: registered.id.clone(),
        name: registered
            .name
            .clone()
            .unwrap_or_else(|| registered.id.clone()),
        provider: registered.provider.clone(),
        api: WireApi::Virtual,
        base_url: String::new(),
        context_window: registered.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
        max_output_tokens: registered.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        reasoning: registered
            .thinking_levels
            .iter()
            .any(|level| level != "off"),
        input,
        output: Vec::new(),
        headers: Default::default(),
        aliases: Vec::new(),
        cost: Default::default(),
        compat: Default::default(),
        thinking,
    }
}

/// List every virtual model the extensions registered. A virtual model hides a physical model of
/// the same provider and id.
pub(crate) fn add_to_catalog(catalog: &mut Catalog, host: Option<&Host>) {
    for registered in host.map(Host::virtual_models).unwrap_or_default() {
        catalog.upsert(catalog_entry(&registered));
    }
}

/// What a router remembers between requests of one session.
#[derive(Default)]
struct Memory {
    /// The request that failed last, by how many messages it carried, and how it failed.
    failed: Option<(usize, Value)>,
    /// The thinking level each physical model was last dispatched with.
    levels: HashMap<String, String>,
}

struct Inner {
    host: Arc<Host>,
    models: ModelRuntime,
    /// The virtual model, as the catalog lists it.
    model: ModelDef,
    session: OnceLock<Arc<tokio::sync::Mutex<micro_session::Session>>>,
    memory: Mutex<Memory>,
}

/// The client a virtual model is selected with: it asks the router where each request goes and
/// hands the request to that model's own client.
#[derive(Clone)]
pub(crate) struct Router {
    /// The provider the virtual model is listed under, which is what the client is called.
    provider: String,
    inner: Arc<Inner>,
}

impl Router {
    pub(crate) fn new(host: Arc<Host>, models: ModelRuntime, model: ModelDef) -> Router {
        Router {
            provider: model.provider.clone(),
            inner: Arc::new(Inner {
                host,
                models,
                model,
                session: OnceLock::new(),
                memory: Mutex::default(),
            }),
        }
    }

    /// The session router state is kept in.
    pub(crate) fn attach_session(&self, session: Arc<tokio::sync::Mutex<micro_session::Session>>) {
        let _ = self.inner.session.set(session);
    }
}

impl Provider for Router {
    fn name(&self) -> &str {
        &self.provider
    }

    fn stream(
        &self,
        model: Model,
        context: Context,
        _api_key: String,
    ) -> UnboundedReceiver<StreamEvent> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            if let Err(message) = inner.dispatch(model, context, &sender).await {
                let _ = sender.send(StreamEvent::Error { message });
            }
        });
        receiver
    }

    /// The router has not chosen yet when the request is written down, so what is recorded is
    /// the selection; the physical model is recorded with the response.
    fn payload(&self, model: &Model, context: &Context) -> Value {
        json!({
            "virtualModel": format!("{}/{}", model.provider, model.id),
            "thinkingLevel": model.thinking.as_str(),
            "messages": context.messages.len(),
        })
    }
}

impl Inner {
    async fn dispatch(
        &self,
        selected: Model,
        context: Context,
        sender: &UnboundedSender<StreamEvent>,
    ) -> Result<(), String> {
        let virtual_id = self.model.qualified_id();
        let count = context.messages.len();
        let failed = self
            .memory
            .lock()
            .ok()
            .and_then(|memory| memory.failed.clone())
            .filter(|(at, _)| *at == count)
            .map(|(_, failed)| failed);
        let reason = match (&failed, context.messages.last()) {
            (Some(_), _) => "retry",
            (None, Some(Message::User { .. })) => "user",
            _ => "continuation",
        };
        let state = self.state().await;

        let mut request = json!({
            "model": micro_models::model_json(&self.model),
            "thinkingLevel": selected.thinking.as_str(),
            "reason": reason,
            "state": state.clone().unwrap_or(Value::Null),
            "messages": self.messages(&context),
        });
        if let Some(previous) = self.previous(&context) {
            request["previous"] = previous;
        }
        if let Some(failed) = failed {
            request["failed"] = failed;
        }

        let decided = self
            .host
            .route(&self.model.provider, &self.model.id, request)
            .await
            .map_err(|error| format!("routing {virtual_id} failed: {error}"))?;
        let provider = decided
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let id = decided
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let level = decided
            .get("thinkingLevel")
            .and_then(Value::as_str)
            .and_then(ThinkingLevel::named)
            .unwrap_or_default();

        let physical = self
            .models
            .find_of_type(ModelType::Chat, provider, id)
            .filter(|model| model.api != WireApi::Virtual)
            .ok_or_else(|| {
                format!("the router of {virtual_id} chose {provider}/{id}, which is not a physical chat model")
            })?;

        if decided.get("stateChanged") == Some(&Value::Bool(true)) {
            self.keep_state(decided.get("state").cloned().unwrap_or(Value::Null))
                .await;
        }

        let key = match self.models.credential(&physical.provider).await {
            Ok(key) => key.unwrap_or_else(|| "local".to_string()),
            Err(error) => {
                return Err(format!(
                    "the router of {virtual_id} chose {}, which has no credential: {error}",
                    physical.qualified_id()
                ))
            }
        };
        let base_url = match micro_auth::canonical_provider(&physical.provider)
            == micro_auth::GITHUB_COPILOT
        {
            true => micro_auth::copilot::base_url_from_token(&key),
            false => None,
        };
        let runtime = crate::runtime::with_host(physical.to_runtime(level), base_url.as_deref());
        let level = runtime.thinking;

        if let Ok(mut memory) = self.memory.lock() {
            memory
                .levels
                .insert(physical.qualified_id(), level.as_str().to_string());
        }

        let client = micro_provider::client_for_model(&physical);
        let mut answered = client.stream(runtime, context, key);
        while let Some(event) = answered.recv().await {
            if let StreamEvent::Error { message } = &event {
                if let Ok(mut memory) = self.memory.lock() {
                    memory.failed = Some((
                        count,
                        json!({
                            "model": micro_models::model_json(&physical),
                            "thinkingLevel": level.as_str(),
                            "message": {
                                "role": "assistant",
                                "provider": physical.provider,
                                "model": physical.id,
                                "content": [],
                                "stopReason": "error",
                                "errorMessage": message,
                            },
                        }),
                    ));
                }
            }
            if let StreamEvent::Done { .. } = &event {
                if let Ok(mut memory) = self.memory.lock() {
                    memory.failed = None;
                }
            }
            if sender.send(event).is_err() {
                break;
            }
        }
        Ok(())
    }

    /// The conversation as the router reads it, system prompt first.
    fn messages(&self, context: &Context) -> Vec<Value> {
        let system = context
            .system_prompt
            .as_ref()
            .map(|prompt| json!({ "role": "system", "content": prompt }));
        system
            .into_iter()
            .chain(context.messages.iter().map(micro_extensions::message_json))
            .collect()
    }

    /// The physical model and level of the latest successful response in the conversation.
    fn previous(&self, context: &Context) -> Option<Value> {
        let answered = context
            .messages
            .iter()
            .rev()
            .find_map(|message| match message {
                Message::Assistant(assistant)
                    if assistant.stop_reason != micro_types::StopReason::Error =>
                {
                    Some(assistant)
                }
                _ => None,
            })?;
        let model =
            self.models
                .find_of_type(ModelType::Chat, &answered.provider, &answered.model)?;
        let level = self
            .memory
            .lock()
            .ok()
            .and_then(|memory| memory.levels.get(&model.qualified_id()).cloned());
        let mut previous = json!({ "model": micro_models::model_json(&model) });
        if let Some(level) = level {
            previous["thinkingLevel"] = json!(level);
        }
        Some(previous)
    }

    /// The router state last kept on the session branch in hand.
    async fn state(&self) -> Option<Value> {
        let session = self.session.get()?;
        let session = session.lock().await;
        let path = session.tree().path_entry_ids();
        let virtual_id = self.model.qualified_id();
        session
            .tree()
            .customs()
            .iter()
            .rev()
            .filter(|custom| custom.custom_type == STATE_ENTRY)
            .filter(|custom| custom.data.get("model").and_then(Value::as_str) == Some(&virtual_id))
            .find(|custom| {
                custom
                    .parent_id
                    .as_ref()
                    .is_none_or(|parent| path.contains(parent))
            })
            .and_then(|custom| custom.data.get("state").cloned())
            .filter(|state| !state.is_null())
    }

    async fn keep_state(&self, state: Value) {
        let Some(session) = self.session.get() else {
            return;
        };
        let kept = session
            .lock()
            .await
            .append_custom(
                STATE_ENTRY,
                json!({ "model": self.model.qualified_id(), "state": state }),
            )
            .await;
        if let Err(error) = kept {
            eprintln!(
                "note: the router state of {} was not kept: {error}",
                self.model.qualified_id()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered(levels: &[&str]) -> RegisteredVirtualModel {
        serde_json::from_value(json!({
            "provider": "router",
            "id": "auto",
            "name": "Auto",
            "thinkingLevels": levels,
        }))
        .unwrap()
    }

    #[test]
    fn a_virtual_model_is_listed_as_a_chat_model_with_the_levels_it_offers() {
        let entry = catalog_entry(&registered(&["low", "high"]));
        assert_eq!(entry.qualified_id(), "router/auto");
        assert_eq!(entry.api, WireApi::Virtual);
        assert_eq!(entry.kind(), ModelType::Chat);
        assert!(entry.reasoning);
        assert_eq!(entry.thinking.get("high"), Some(&Some("high".to_string())));
        assert_eq!(entry.thinking.get("medium"), Some(&None));
        assert_eq!(entry.input, vec![Modality::Text, Modality::Image]);
        assert_eq!(entry.context_window, DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn a_virtual_model_that_offers_only_off_does_not_reason() {
        let entry =
            catalog_entry(&serde_json::from_value(json!({ "provider": "x", "id": "y" })).unwrap());
        assert!(!entry.reasoning);
        assert_eq!(entry.name, "y");
    }

    #[test]
    fn a_virtual_model_hides_a_physical_model_with_its_name() {
        let mut catalog = Catalog::bundled();
        let physical = catalog.models()[0].clone();
        catalog.upsert(catalog_entry(
            &serde_json::from_value(json!({ "provider": physical.provider, "id": physical.id }))
                .unwrap(),
        ));
        assert_eq!(
            catalog.get(&physical.provider, &physical.id).unwrap().api,
            WireApi::Virtual
        );
    }

    #[test]
    fn a_virtual_model_offers_exactly_the_levels_it_declares() {
        let declared = |levels: &[&str]| {
            catalog_entry(
                &serde_json::from_value(json!({
                    "provider": "router",
                    "id": "auto",
                    "thinkingLevels": levels,
                }))
                .unwrap(),
            )
            .thinking_levels()
        };
        assert_eq!(declared(&["off"]), vec![ThinkingLevel::Off]);
        assert_eq!(
            declared(&["low", "high", "xhigh"]),
            vec![
                ThinkingLevel::Low,
                ThinkingLevel::High,
                ThinkingLevel::XHigh
            ]
        );
    }
}
