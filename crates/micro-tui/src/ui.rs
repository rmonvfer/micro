//! What the interface is asked to do from outside it.

use serde_json::json;
use serde_json::Value;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

/// What is being asked for, and where the answer goes.
#[derive(Debug)]
pub struct UiRequest {
    /// `select`, `confirm`, `input`, `notify`, `setStatus`, or one of the other things an extension
    /// can ask the interface to do; see.
    pub method: String,
    /// What the question says.
    pub title: String,
    /// The body of a confirmation, or the placeholder of an input.
    pub detail: Option<String>,
    /// What may be chosen, for a selection.
    pub options: Vec<String>,
    
    pub extension: Option<String>,
    answer: Option<oneshot::Sender<Value>>,
}

impl UiRequest {
    /// Give the answer.
    pub fn answer(&mut self, value: Value) {
        if let Some(sender) = self.answer.take() {
            let _ = sender.send(value);
        }
    }

    
    pub fn cancel(&mut self) {
        self.answer(json!({ "cancelled": true }));
    }

    
    #[cfg(test)]
    pub fn for_test(
        method: impl Into<String>,
        title: impl Into<String>,
        detail: Option<String>,
        options: Vec<String>,
    ) -> (UiRequest, oneshot::Receiver<Value>) {
        UiRequest::for_test_from(None, method, title, detail, options)
    }

    /// The same, from a named extension, for a test about what letting one go takes back.
    #[cfg(test)]
    pub fn for_test_from(
        extension: Option<String>,
        method: impl Into<String>,
        title: impl Into<String>,
        detail: Option<String>,
        options: Vec<String>,
    ) -> (UiRequest, oneshot::Receiver<Value>) {
        let (sender, receiver) = oneshot::channel();
        (
            UiRequest {
                method: method.into(),
                title: title.into(),
                detail,
                options,
                extension,
                answer: Some(sender),
            },
            receiver,
        )
    }
}

impl Drop for UiRequest {
    
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The end that asks.
#[derive(Debug, Clone)]
pub struct UiAsker(UnboundedSender<UiRequest>);

impl UiAsker {
    /// Ask, and wait for the answer.
    pub async fn ask(
        &self,
        method: impl Into<String>,
        title: impl Into<String>,
        detail: Option<String>,
        options: Vec<String>,
    ) -> Value {
        self.ask_from(None, method, title, detail, options).await
    }

    /// Ask on a named extension's behalf, for the asks that leave something on the screen after the
    /// call returns.
    pub async fn ask_from(
        &self,
        extension: Option<String>,
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
            extension,
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


#[derive(Debug)]
pub struct TerminalInputAsk {
    /// The key, written the way a terminal would have sent it.
    pub data: String,
    answer: Option<oneshot::Sender<Value>>,
}

impl TerminalInputAsk {
    /// Say what an extension decided.
    pub fn answer(&mut self, value: Value) {
        if let Some(sender) = self.answer.take() {
            let _ = sender.send(value);
        }
    }
}

impl Drop for TerminalInputAsk {
    /// Nobody answering is nobody consuming it: the key still reaches the editor.
    fn drop(&mut self) {
        if let Some(sender) = self.answer.take() {
            let _ = sender.send(json!({}));
        }
    }
}

/// The end that asks: the interface, once per keystroke while an extension is listening.
#[derive(Debug, Clone)]
pub struct TerminalInputAsker(UnboundedSender<TerminalInputAsk>);

impl TerminalInputAsker {
    /// Offer a key, and wait to hear whether it was consumed.
    pub async fn ask(&self, data: String) -> Value {
        let (sender, receiver) = oneshot::channel();
        let ask = TerminalInputAsk {
            data,
            answer: Some(sender),
        };
        if self.0.send(ask).is_err() {
            return json!({});
        }
        receiver.await.unwrap_or_else(|_| json!({}))
    }
}

/// The end that answers: whatever is relaying keys to the extension host.
#[derive(Debug)]
pub struct TerminalInputAsks(UnboundedReceiver<TerminalInputAsk>);

impl TerminalInputAsks {
    pub async fn recv(&mut self) -> Option<TerminalInputAsk> {
        self.0.recv().await
    }
}

/// Both ends of the path a keystroke takes to whoever is listening for it.
pub fn terminal_input_channel() -> (TerminalInputAsker, TerminalInputAsks) {
    let (sender, receiver) = unbounded_channel();
    (TerminalInputAsker(sender), TerminalInputAsks(receiver))
}


#[derive(Debug)]
pub struct HostAsk {
    /// What is being asked.
    pub event: String,
    pub payload: Value,
    answer: Option<oneshot::Sender<Value>>,
}

impl HostAsk {
    /// Give the answer.
    pub fn answer(&mut self, value: Value) {
        if let Some(sender) = self.answer.take() {
            let _ = sender.send(value);
        }
    }
}

impl Drop for HostAsk {
    /// Nobody answering an ask is answered as an empty object.
    fn drop(&mut self) {
        self.answer(json!({}));
    }
}


#[derive(Debug, Clone)]
pub struct HostAsker(UnboundedSender<HostAsk>);

impl HostAsker {
    /// Ask, and wait for the answer.
    pub async fn ask(&self, event: impl Into<String>, payload: Value) -> Value {
        let (sender, receiver) = oneshot::channel();
        let ask = HostAsk {
            event: event.into(),
            payload,
            answer: Some(sender),
        };
        if self.0.send(ask).is_err() {
            return json!({});
        }
        receiver.await.unwrap_or_else(|_| json!({}))
    }
}

/// The end that answers: whatever is relaying asks to the extension host.
#[derive(Debug)]
pub struct HostAsks(UnboundedReceiver<HostAsk>);

impl HostAsks {
    pub async fn recv(&mut self) -> Option<HostAsk> {
        self.0.recv().await
    }
}

/// Both ends of the path an off-frame question to the host takes.
pub fn host_ask_channel() -> (HostAsker, HostAsks) {
    let (sender, receiver) = unbounded_channel();
    (HostAsker(sender), HostAsks(receiver))
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

        let asking =
            tokio::spawn(async move { asker.ask("confirm", "sure?", None, Vec::new()).await });

        
        drop(requests.recv().await.expect("a question"));
        assert_eq!(asking.await.unwrap()["cancelled"], true);
    }

    #[tokio::test]
    async fn asking_an_interface_that_has_gone_is_answered_at_once() {
        let (asker, requests) = ui_channel();
        drop(requests);
        assert_eq!(
            asker.ask("input", "name?", None, Vec::new()).await["cancelled"],
            true
        );
    }

    #[tokio::test]
    async fn a_key_reaches_whoever_is_listening_and_the_verdict_comes_back() {
        let (asker, mut asks) = terminal_input_channel();

        let asking = tokio::spawn(async move { asker.ask("j".to_string()).await });

        let mut ask = asks.recv().await.expect("a key");
        assert_eq!(ask.data, "j");
        ask.answer(json!({ "consume": true }));

        assert_eq!(asking.await.unwrap()["consume"], true);
    }

    /// Nothing answering a key is nothing consuming it, so the editor still gets to.
    #[tokio::test]
    async fn a_key_nobody_answers_is_not_consumed() {
        let (asker, mut asks) = terminal_input_channel();

        let asking = tokio::spawn(async move { asker.ask("x".to_string()).await });
        drop(asks.recv().await.expect("a key"));

        assert!(asking.await.unwrap().get("consume").is_none());
    }

    #[tokio::test]
    async fn a_key_offered_to_nobody_listening_is_not_consumed() {
        let (asker, asks) = terminal_input_channel();
        drop(asks);
        assert!(asker.ask("x".to_string()).await.get("consume").is_none());
    }

    #[tokio::test]
    async fn an_off_frame_ask_reaches_the_host_and_the_answer_comes_back() {
        let (asker, mut asks) = host_ask_channel();

        let asking = tokio::spawn(async move {
            asker
                .ask("component_input", json!({ "id": "c1", "data": "x" }))
                .await
        });

        let mut ask = asks.recv().await.expect("an ask");
        assert_eq!(ask.event, "component_input");
        assert_eq!(ask.payload["id"], "c1");
        ask.answer(json!({ "lines": ["hi"] }));

        assert_eq!(asking.await.unwrap()["lines"][0], "hi");
    }

    #[tokio::test]
    async fn an_off_frame_ask_nobody_answers_comes_back_empty() {
        let (asker, mut asks) = host_ask_channel();
        let asking = tokio::spawn(async move { asker.ask("get_suggestions", json!({})).await });
        drop(asks.recv().await.expect("an ask"));
        assert_eq!(asking.await.unwrap(), json!({}));
    }
}
