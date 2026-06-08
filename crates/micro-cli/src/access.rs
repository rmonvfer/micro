//! Interactive approval for narrowly scoped sandbox access requests.

use async_trait::async_trait;
use micro_tools::AccessApproval;
use micro_tools::AccessApprover;
use micro_tools::AccessRequest;

pub struct TerminalAccessApprover {
    asker: micro_tui::UiAsker,
}

impl TerminalAccessApprover {
    pub fn new(asker: micro_tui::UiAsker) -> Self {
        TerminalAccessApprover { asker }
    }
}

#[async_trait]
impl AccessApprover for TerminalAccessApprover {
    async fn approve(&self, request: AccessRequest) -> AccessApproval {
        let answer = self
            .asker
            .ask(
                "sandbox_access",
                format!("Allow {} access?", request.capability.name()),
                Some(format!(
                    "{}\n\nCommand: {}",
                    request.reason, request.command
                )),
                vec![
                    "Allow once".into(),
                    "Allow for this session".into(),
                    "Deny".into(),
                ],
            )
            .await;
        match answer.get("value").and_then(serde_json::Value::as_str) {
            Some("Allow once") => AccessApproval::Once,
            Some("Allow for this session") => AccessApproval::Session,
            _ => AccessApproval::Denied,
        }
    }
}
