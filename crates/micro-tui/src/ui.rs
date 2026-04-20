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
    /// `select`, `confirm`, `input`, `notify`, `setStatus`, or one of the other things an
    /// extension can ask the interface to do; see [`crate::app::App::ask_question`] for the
    /// whole set.
    pub method: String,
    /// What the question says.
    pub title: String,
    /// The body of a confirmation, or the placeholder of an input.
    pub detail: Option<String>,
    /// What may be chosen, for a selection.
    pub options: Vec<String>,
    /// The extension that asked, where the ask carried one — the file it was loaded from,
    /// which is how the host names it. Set only on the asks whose effect outlives the call:
    /// a status line, a widget, a header, a footer, an editor. What an extension leaves on
    /// the screen has to be attributable to it, or letting it go could not take it back.
    pub extension: Option<String>,
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

    /// Build one directly, with somewhere to read the answer from, for a test that wants to
    /// drive [`crate::app::App::ask_question`] without a channel and a second task to run
    /// [`UiAsker::ask`] on. Mirrors `KeyPrompt::for_test` in `app.rs` for the same reason.
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
        self.ask_from(None, method, title, detail, options).await
    }

    /// Ask on a named extension's behalf, for the asks that leave something on the screen
    /// after the call returns — see [`UiRequest::extension`].
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

/// A key the interface read, offered to whatever an extension registered with
/// `ctx.ui.onTerminalInput` before the interface acts on it itself.
///
/// The direction is the reverse of [`UiRequest`]: there the host asks the interface
/// something, here the interface asks the host. Kept as its own small channel rather than
/// folded into the one above, since nothing but a key press ever travels this way and
/// nothing but this needs to run once per keystroke.
#[derive(Debug)]
pub struct TerminalInputAsk {
    /// The key, written the way a terminal would have sent it — a printable character as
    /// itself, a control key as the escape sequence a terminal emits for it. Not
    /// necessarily the exact bytes crossterm read, since crossterm keeps only the key it
    /// parsed them into; see [`crate::event::key_to_data`] for what is reconstructed.
    pub data: String,
    answer: Option<oneshot::Sender<Value>>,
}

impl TerminalInputAsk {
    /// Say what an extension decided. A key answered twice keeps the first answer.
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
    /// Offer a key, and wait to hear whether it was consumed. Nobody left to ask answers
    /// at once, the same way an interface that has gone answers a closed [`UiAsker`].
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

/// Something the interface needs from the host off its own initiative, outside a frame: a
/// keystroke for whichever component has focus, a completion list for the menu. One shape
/// for every use rather than a channel apiece, since every use is alike — a name for what
/// is being asked, whatever the host needs to answer it, and somewhere to send the answer.
///
/// A frame never waits on this: whatever asks it runs off the render path, in whatever
/// handles the keystroke or opens the menu, and draws with whatever it already has until
/// the answer lands.
#[derive(Debug)]
pub struct HostAsk {
    /// What is being asked — `"component_input"`, `"get_suggestions"`, and so on; see
    /// `crates/micro-extensions/host/ui.ts`'s `handle` for the names it answers.
    pub event: String,
    pub payload: Value,
    answer: Option<oneshot::Sender<Value>>,
}

impl HostAsk {
    /// Give the answer. An ask answered twice keeps the first answer.
    pub fn answer(&mut self, value: Value) {
        if let Some(sender) = self.answer.take() {
            let _ = sender.send(value);
        }
    }
}

impl Drop for HostAsk {
    /// Nobody answering an ask is answered as an empty object, which is what every reader
    /// of one treats as "nothing to add" rather than waiting forever.
    fn drop(&mut self) {
        self.answer(json!({}));
    }
}

/// The end that asks: the interface, whenever it needs something from the host that is not
/// worth holding a frame for.
#[derive(Debug, Clone)]
pub struct HostAsker(UnboundedSender<HostAsk>);

impl HostAsker {
    /// Ask, and wait for the answer. Nobody left to ask answers with an empty object at
    /// once, the same way a key offered to nobody is answered as not consumed.
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

        // Dropped rather than answered, which is what closing the overlay does.
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
