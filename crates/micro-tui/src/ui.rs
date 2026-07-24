//! What the interface is asked to do from outside it.
//!
//! An extension runs in another process and cannot draw anything. When it wants the user
//! asked something it sends the question here, and waits: the interface opens whatever
//! suits the question, and the answer goes back down the same path.
//!
//! Modelled on [`crate::approval`] for the same reason — the asker and the answerer are in
//! different places, and neither should know how the other works.

use serde_json::json;
use serde_json::Value;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

/// What is being asked for, and where the answer goes.
///
/// Mostly a question for the user, but the same path carries anything only the interface
/// can do — putting a message into the conversation among them.
#[derive(Debug)]
pub struct UiRequest {
    /// `select`, `confirm`, `input`, `notify`, or `send_user_message`.
    pub method: String,
    /// What the question says.
    pub title: String,
    /// The body of a confirmation, or the placeholder of an input.
    pub detail: Option<String>,
    /// What may be chosen, for a selection.
    pub options: Vec<String>,
    answer: Option<oneshot::Sender<Value>>,
}

impl UiRequest {
    /// Give the answer. A question answered twice keeps the first answer.
    pub fn answer(&mut self, value: Value) {
        if let Some(sender) = self.answer.take() {
            let _ = sender.send(value);
        }
    }

    /// Say that nobody answered, which is what closing a question means.
    pub fn cancel(&mut self) {
        self.answer(json!({ "cancelled": true }));
    }
}

impl Drop for UiRequest {
    /// A question dropped without an answer is a question nobody answered, and the asker
    /// is told so rather than left waiting.
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The end that asks.
#[derive(Debug, Clone)]
pub struct UiAsker(UnboundedSender<UiRequest>);

impl UiAsker {
    /// Ask, and wait for the answer. A closed interface answers `cancelled` at once.
    pub async fn ask(
        &self,
        method: impl Into<String>,
        title: impl Into<String>,
        detail: Option<String>,
        options: Vec<String>,
    ) -> Value {
        let (sender, receiver) = oneshot::channel();
        let request = UiRequest {
            method: method.into(),
            title: title.into(),
            detail,
            options,
            answer: Some(sender),
        };
        if self.0.send(request).is_err() {
            return json!({ "cancelled": true });
        }
        receiver
            .await
            .unwrap_or_else(|_| json!({ "cancelled": true }))
    }
}

/// The end that answers.
#[derive(Debug)]
pub struct UiRequests(UnboundedReceiver<UiRequest>);

impl UiRequests {
    pub async fn recv(&mut self) -> Option<UiRequest> {
        self.0.recv().await
    }

    pub fn try_recv(&mut self) -> Option<UiRequest> {
        self.0.try_recv().ok()
    }
}

/// Both ends of the path a question takes.
pub fn ui_channel() -> (UiAsker, UiRequests) {
    let (sender, receiver) = unbounded_channel();
    (UiAsker(sender), UiRequests(receiver))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_question_reaches_the_interface_and_the_answer_comes_back() {
        let (asker, mut requests) = ui_channel();

        let asking = tokio::spawn(async move {
            asker
                .ask("select", "pick one", None, vec!["a".into(), "b".into()])
                .await
        });

        let mut request = requests.recv().await.expect("a question");
        assert_eq!(request.method, "select");
        assert_eq!(request.title, "pick one");
        assert_eq!(request.options, vec!["a", "b"]);
        request.answer(json!({ "value": "b" }));

        assert_eq!(asking.await.unwrap()["value"], "b");
    }

    #[tokio::test]
    async fn a_question_nobody_answers_comes_back_cancelled() {
        let (asker, mut requests) = ui_channel();

        let asking = tokio::spawn(async move { asker.ask("confirm", "sure?", None, Vec::new()).await });

        // Dropped rather than answered, which is what closing the overlay does.
        drop(requests.recv().await.expect("a question"));
        assert_eq!(asking.await.unwrap()["cancelled"], true);
    }

    #[tokio::test]
    async fn asking_an_interface_that_has_gone_is_answered_at_once() {
        let (asker, requests) = ui_channel();
        drop(requests);
        assert_eq!(asker.ask("input", "name?", None, Vec::new()).await["cancelled"], true);
    }
}
