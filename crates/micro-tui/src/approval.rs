//! Asking the user whether a tool may run.
//!
//! The policy calls [`Approver::approve`] from the agent's task, while the interface's own
//! loop owns the terminal. The two are joined by a pair of channels: the request goes out on
//! one, the answer comes back on the other, and the loop keeps painting in between.
//!
//! Nothing here can leave the agent waiting forever. Every path that loses the ability to
//! answer — the interface closing, a turn being interrupted, a queue dropped mid-question —
//! resolves the call to [`Approval::Denied`] with a message the model can act on.

use async_trait::async_trait;
use micro_policy::Approval;
use micro_policy::ApprovalRequest;
use micro_policy::Approver;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

/// Refusal for a call nobody is left to answer.
pub(crate) const UNANSWERABLE: &str = "the user could not be asked, because the interface is \
                                       no longer listening; treat this as a refusal";
/// Refusal for a call the user cut short.
pub(crate) const INTERRUPTED: &str = "the user interrupted before answering; treat this as a \
                                      refusal and wait for their next instruction";
/// Refusal for a call the user turned down.
pub(crate) const DECLINED: &str = "the user declined to allow this; do not try it again, and \
                                   consider whether another approach would serve them better";

/// A request on its way to the user, and the return path for the answer.
#[derive(Debug)]
pub struct PendingApproval {
    request: ApprovalRequest,
    reply: oneshot::Sender<Approval>,
}

impl PendingApproval {
    /// Pair a request with the channel its answer travels back on.
    pub(crate) fn new(request: ApprovalRequest) -> (Self, oneshot::Receiver<Approval>) {
        let (reply, answer) = oneshot::channel();
        (PendingApproval { request, reply }, answer)
    }

    pub fn request(&self) -> &ApprovalRequest {
        &self.request
    }

    /// Send the answer. A closed return path means the caller gave up first, which is not
    /// this side's problem to report.
    fn answer(self, approval: Approval) {
        let _ = self.reply.send(approval);
    }
}

/// The [`Approver`] to hand to the policy engine.
#[derive(Debug)]
pub struct ApprovalGate {
    requests: UnboundedSender<PendingApproval>,
}

#[async_trait]
impl Approver for ApprovalGate {
    async fn approve(&self, request: &ApprovalRequest) -> Approval {
        let (pending, answer) = PendingApproval::new(request.clone());
        if self.requests.send(pending).is_err() {
            return Approval::Denied(UNANSWERABLE.to_string());
        }
        answer
            .await
            .unwrap_or_else(|_| Approval::Denied(UNANSWERABLE.to_string()))
    }
}

/// The interface's end of the pair: requests arrive here to be put on screen.
#[derive(Debug)]
pub struct ApprovalRequests(UnboundedReceiver<PendingApproval>);

impl ApprovalRequests {
    pub(crate) async fn recv(&mut self) -> Option<PendingApproval> {
        self.0.recv().await
    }

    /// Whatever has already arrived, without waiting for more.
    pub(crate) fn try_recv(&mut self) -> Option<PendingApproval> {
        self.0.try_recv().ok()
    }
}

/// Build the pair joining the policy to the interface.
///
/// Give the approver to `micro_policy::PolicyEngine::new` and the requests to
/// [`crate::TuiOptions::approvals`]; the interface then asks instead of refusing.
pub fn approval_channel() -> (Arc<dyn Approver>, ApprovalRequests) {
    let (sender, receiver) = unbounded_channel();
    (
        Arc::new(ApprovalGate { requests: sender }),
        ApprovalRequests(receiver),
    )
}

/// What the user can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Choice {
    #[default]
    Once,
    Session,
    Deny,
}

impl Choice {
    /// The three answers in the order they are offered.
    pub const ALL: [Choice; 3] = [Choice::Once, Choice::Session, Choice::Deny];

    pub fn label(&self) -> &'static str {
        match self {
            Choice::Once => "allow once",
            Choice::Session => "allow for this session",
            Choice::Deny => "decline",
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Choice::Once => "y",
            Choice::Session => "a",
            Choice::Deny => "n",
        }
    }

    /// The answer a typed character stands for.
    pub fn from_key(text: &str) -> Option<Choice> {
        match text.trim().to_ascii_lowercase().as_str() {
            "y" => Some(Choice::Once),
            "a" => Some(Choice::Session),
            "n" => Some(Choice::Deny),
            _ => None,
        }
    }

    fn approval(&self) -> Approval {
        match self {
            Choice::Once => Approval::Once,
            Choice::Session => Approval::Session,
            Choice::Deny => Approval::Denied(DECLINED.to_string()),
        }
    }
}

/// Requests waiting on the user, shown one at a time in the order they arrived.
#[derive(Debug, Default)]
pub struct ApprovalQueue {
    showing: Option<PendingApproval>,
    waiting: VecDeque<PendingApproval>,
    selected: Choice,
}

impl ApprovalQueue {
    pub fn new() -> Self {
        ApprovalQueue::default()
    }

    /// True while a request is on screen and the keyboard belongs to it.
    pub fn is_open(&self) -> bool {
        self.showing.is_some()
    }

    pub fn showing(&self) -> Option<&ApprovalRequest> {
        self.showing.as_ref().map(PendingApproval::request)
    }

    /// How many more requests are behind the one on screen.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    pub fn selected(&self) -> Choice {
        self.selected
    }

    /// Take a request. It goes on screen if nothing else is, and in line behind whatever is.
    pub fn push(&mut self, pending: PendingApproval) {
        match self.showing {
            Some(_) => self.waiting.push_back(pending),
            None => self.show(Some(pending)),
        }
    }

    pub fn select_next(&mut self) {
        self.move_selection(1);
    }

    pub fn select_previous(&mut self) {
        self.move_selection(Choice::ALL.len() - 1);
    }

    /// Answer with whatever is highlighted.
    pub fn confirm(&mut self) {
        self.answer(self.selected);
    }

    /// Answer the request on screen and bring up the next one.
    pub fn answer(&mut self, choice: Choice) {
        let Some(pending) = self.showing.take() else {
            return;
        };
        pending.answer(choice.approval());
        let next = self.waiting.pop_front();
        self.show(next);
    }

    /// Refuse everything outstanding. `reason` is what the model reads as the tool error.
    pub fn deny_all(&mut self, reason: &str) {
        let mut outstanding: Vec<PendingApproval> = self.showing.take().into_iter().collect();
        outstanding.extend(self.waiting.drain(..));
        for pending in outstanding {
            pending.answer(Approval::Denied(reason.to_string()));
        }
        self.selected = Choice::default();
    }

    /// Put a request on screen, starting its selection from the default rather than from
    /// whatever the previous request happened to be left on.
    fn show(&mut self, pending: Option<PendingApproval>) {
        self.showing = pending;
        self.selected = Choice::default();
    }

    fn move_selection(&mut self, by: usize) {
        let current = Choice::ALL
            .iter()
            .position(|choice| *choice == self.selected)
            .unwrap_or(0);
        self.selected = Choice::ALL[(current + by) % Choice::ALL.len()];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(subject: &str) -> ApprovalRequest {
        ApprovalRequest {
            tool: "bash".into(),
            subject: Some(subject.into()),
            arguments: json!({ "command": subject }),
            reason: "policy asks before running a shell command".into(),
            key: format!("bash:{subject}"),
        }
    }

    /// A queued request paired with the answer channel a caller would be awaiting.
    fn pending(subject: &str) -> (PendingApproval, oneshot::Receiver<Approval>) {
        PendingApproval::new(request(subject))
    }

    #[test]
    fn a_queue_starts_closed() {
        let queue = ApprovalQueue::new();
        assert!(!queue.is_open());
        assert_eq!(queue.showing(), None);
        assert_eq!(queue.waiting(), 0);
    }

    #[test]
    fn the_first_request_goes_straight_on_screen() {
        let mut queue = ApprovalQueue::new();
        let (pending, _answer) = pending("ls");
        queue.push(pending);

        assert!(queue.is_open());
        assert_eq!(queue.showing().unwrap().subject.as_deref(), Some("ls"));
        assert_eq!(queue.waiting(), 0);
    }

    #[test]
    fn requests_are_answered_one_at_a_time_in_arrival_order() {
        let mut queue = ApprovalQueue::new();
        let mut answers = Vec::new();
        for subject in ["first", "second", "third"] {
            let (pending, answer) = pending(subject);
            queue.push(pending);
            answers.push(answer);
        }

        assert_eq!(queue.showing().unwrap().subject.as_deref(), Some("first"));
        assert_eq!(queue.waiting(), 2);

        queue.answer(Choice::Once);
        assert_eq!(queue.showing().unwrap().subject.as_deref(), Some("second"));
        assert_eq!(queue.waiting(), 1);

        queue.answer(Choice::Deny);
        assert_eq!(queue.showing().unwrap().subject.as_deref(), Some("third"));

        queue.answer(Choice::Session);
        assert!(!queue.is_open());

        let received: Vec<Approval> = answers
            .into_iter()
            .map(|mut answer| answer.try_recv().expect("an answer was sent"))
            .collect();
        assert_eq!(received[0], Approval::Once);
        assert!(matches!(received[1], Approval::Denied(_)));
        assert_eq!(received[2], Approval::Session);
    }

    #[test]
    fn answering_an_empty_queue_does_nothing() {
        let mut queue = ApprovalQueue::new();
        queue.answer(Choice::Once);
        assert!(!queue.is_open());
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut queue = ApprovalQueue::new();
        assert_eq!(queue.selected(), Choice::Once);

        queue.select_next();
        assert_eq!(queue.selected(), Choice::Session);
        queue.select_next();
        assert_eq!(queue.selected(), Choice::Deny);
        queue.select_next();
        assert_eq!(queue.selected(), Choice::Once);

        queue.select_previous();
        assert_eq!(queue.selected(), Choice::Deny);
    }

    #[test]
    fn confirming_answers_with_the_highlighted_choice() {
        let mut queue = ApprovalQueue::new();
        let (pending, mut answer) = pending("ls");
        queue.push(pending);
        queue.select_next();
        queue.confirm();

        assert_eq!(answer.try_recv().unwrap(), Approval::Session);
    }

    #[test]
    fn the_next_request_starts_from_the_default_choice() {
        let mut queue = ApprovalQueue::new();
        let (first, _first) = pending("first");
        let (second, _second) = pending("second");
        queue.push(first);
        queue.push(second);

        queue.select_next();
        queue.answer(Choice::Once);
        assert_eq!(
            queue.selected(),
            Choice::Once,
            "a highlight does not carry over to the next question"
        );
    }

    #[test]
    fn denying_everything_answers_the_whole_queue() {
        let mut queue = ApprovalQueue::new();
        let mut answers = Vec::new();
        for subject in ["first", "second"] {
            let (pending, answer) = pending(subject);
            queue.push(pending);
            answers.push(answer);
        }

        queue.deny_all(INTERRUPTED);
        assert!(!queue.is_open());
        assert_eq!(queue.waiting(), 0);

        for mut answer in answers {
            let Approval::Denied(reason) = answer.try_recv().unwrap() else {
                panic!("expected a refusal");
            };
            assert_eq!(reason, INTERRUPTED);
        }
    }

    #[test]
    fn keys_map_to_the_answers_they_show() {
        assert_eq!(Choice::from_key("y"), Some(Choice::Once));
        assert_eq!(Choice::from_key("A"), Some(Choice::Session));
        assert_eq!(Choice::from_key("n"), Some(Choice::Deny));
        assert_eq!(Choice::from_key("q"), None);
        assert_eq!(Choice::from_key("yes"), None, "a paste is not an answer");

        for choice in Choice::ALL {
            assert_eq!(Choice::from_key(choice.key()), Some(choice));
        }
    }

    #[tokio::test]
    async fn an_approval_travels_to_the_queue_and_the_answer_comes_back() {
        let (approver, mut requests) = approval_channel();
        let asked = tokio::spawn(async move { approver.approve(&request("ls")).await });

        let pending = requests.recv().await.expect("the request arrived");
        let mut queue = ApprovalQueue::new();
        queue.push(pending);
        queue.answer(Choice::Session);

        assert_eq!(asked.await.unwrap(), Approval::Session);
    }

    #[tokio::test]
    async fn a_closed_interface_refuses_rather_than_hanging() {
        let (approver, requests) = approval_channel();
        drop(requests);

        let Approval::Denied(reason) = approver.approve(&request("ls")).await else {
            panic!("expected a refusal");
        };
        assert_eq!(reason, UNANSWERABLE);
    }

    #[tokio::test]
    async fn a_queue_dropped_mid_question_refuses_rather_than_hanging() {
        let (approver, mut requests) = approval_channel();
        let asked = tokio::spawn(async move { approver.approve(&request("ls")).await });

        let pending = requests.recv().await.expect("the request arrived");
        let mut queue = ApprovalQueue::new();
        queue.push(pending);
        // The interface goes away without answering, which is what a panic would look like.
        drop(queue);

        let Approval::Denied(reason) = asked.await.unwrap() else {
            panic!("expected a refusal");
        };
        assert_eq!(reason, UNANSWERABLE);
    }
}
