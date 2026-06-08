use micro_extensions::Capability;
use micro_extensions::Grants;
use serde_json::Value;
use std::sync::Arc;

/// Where a fact about what an extension asked for goes.
pub type Crossings = tokio::sync::mpsc::WeakUnboundedSender<micro_types::LedgerEvent>;

/// Decides extension capabilities and records every capability boundary crossing.
#[derive(Clone)]
pub struct Broker {
    pub grants: Arc<Grants>,
    pub crossings: Option<Crossings>,
}

impl Broker {
    /// A broker that permits everything and records nothing, for a caller with no extensions
    /// loaded.
    pub fn open() -> Broker {
        Broker {
            grants: Arc::new(Grants::default()),
            crossings: None,
        }
    }

    pub(crate) fn allows(&self, extension: Option<&str>, needs: Capability, name: &str) -> bool {
        let allowed = self.grants.allows(extension, needs);

        if allowed && matches!(needs, Capability::Ui) {
            return true;
        }
        self.record(extension, needs.as_str(), name, allowed, None);
        allowed
    }

    pub(crate) fn record(
        &self,
        extension: Option<&str>,
        kind: &str,
        name: &str,
        allowed: bool,
        detail: Option<Value>,
    ) {
        let Some(crossings) = self
            .crossings
            .as_ref()
            .and_then(|crossings| crossings.upgrade())
        else {
            return;
        };

        let Some(extension) = extension else {
            return;
        };
        let _ = crossings.send(micro_types::LedgerEvent::ExtensionCrossing {
            extension: self.grants.name_of(Some(extension)),
            kind: kind.to_string(),
            name: name.to_string(),
            allowed,
            detail,
        });
    }

    /// What an extension is told when it asked for something it may not do.
    pub(crate) fn refusal(&self, extension: Option<&str>, needs: Capability) -> String {
        format!(
            "capability '{}' not granted to {}",
            needs,
            self.grants.name_of(extension)
        )
    }

    /// Which of an event's answers may change what micro does.
    pub(crate) fn heeded(
        &self,
        answers: Vec<(Option<String>, Value)>,
        needs: Capability,
        name: &str,
    ) -> Vec<Value> {
        self.heeded_from(answers, needs, name)
            .into_iter()
            .map(|(_, answer)| answer)
            .collect()
    }

    pub(crate) fn heeded_from(
        &self,
        answers: Vec<(Option<String>, Value)>,
        needs: Capability,
        name: &str,
    ) -> Vec<(Option<String>, Value)> {
        answers
            .into_iter()
            .filter(|(source, _)| {
                let allowed = self.grants.allows(source.as_deref(), needs);
                if !allowed {
                    self.record(source.as_deref(), needs.as_str(), name, false, None);
                }
                allowed
            })
            .collect()
    }
}

/// The capability a request needs, or nothing when it only reads.
pub(crate) fn request_needs(request: &str) -> Option<Capability> {
    match request {
        "exec" => Some(Capability::Exec),
        "run_builtin_tool" => Some(Capability::BuiltinTools),
        "provider_stream" => Some(Capability::ProviderStream),
        "append_entry" | "set_label" | "set_session_name" => Some(Capability::SessionWrite),
        "set_model" => Some(Capability::SessionControl),
        "reload" | "new_session" | "switch_session" | "navigate_tree" | "fork" => {
            Some(Capability::SessionControl)
        }
        _ => None,
    }
}

/// The capability an action needs, or nothing when micro does not know the action at all.
pub(crate) fn action_needs(action: &str) -> Option<Capability> {
    match action {
        "send_user_message" => Some(Capability::SendUserMessage),
        "send_message" => Some(Capability::SendMessage),
        "set_active_tools" => Some(Capability::Context),
        "append_entry" | "set_label" | "set_session_name" => Some(Capability::SessionWrite),
        "set_thinking_level" | "set_model" | "shutdown" | "compact" | "abort" => {
            Some(Capability::SessionControl)
        }
        "watch_terminal_input" | "unwatch_terminal_input" | "watch_autocomplete" => {
            Some(Capability::Ui)
        }
        _ => None,
    }
}
