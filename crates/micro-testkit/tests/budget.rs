//! What a run does when it has spent what it was allowed to.

use std::sync::Arc;

use micro_agent::Agent;
use micro_agent::Budget;
use micro_agent::Record;
use micro_models::ModelCost;
use micro_testkit::run_agent;
use micro_testkit::FakeProvider;
use micro_testkit::FakeTool;
use micro_testkit::Turn;
use micro_types::LedgerEvent;
use micro_types::Message;
use micro_types::Model;
use micro_types::StopReason;
use micro_types::ThinkingLevel;
use micro_types::Usage;
use serde_json::json;

fn model() -> Model {
    Model {
        id: "test-model".into(),
        provider: "fake".into(),
        base_url: "https://example.invalid".into(),
        max_tokens: 1024,
        thinking: ThinkingLevel::Off,
        reasoning: Default::default(),
        compat: Default::default(),
        headers: Default::default(),
    }
}

/// Dollars per million tokens, chosen so one scripted turn costs exactly one cent.
fn rates() -> ModelCost {
    ModelCost {
        input: 5.0,
        output: 5.0,
        cache_read: 0.0,
        cache_write: 0.0,
        tiers: Vec::new(),
    }
}

/// A turn billed at a cent: a thousand tokens in and a thousand out, at the rates above.
fn a_cent() -> Usage {
    Usage {
        input: 1_000,
        output: 1_000,
        cache_read: 0,
        cache_write: 0,
    }
}

/// A run that asks for a tool and then answers, which is two requests when nothing stops it.
fn two_turns() -> FakeProvider {
    FakeProvider::builder()
        .turn(
            Turn::text("looking")
                .with_tool_call("call_1", "read", json!({ "path": "a.txt" }))
                .with_usage(a_cent()),
        )
        .turn(Turn::text("done").with_usage(a_cent()))
        .build()
}

fn events(recorded: &mut tokio::sync::mpsc::UnboundedReceiver<Record>) -> Vec<LedgerEvent> {
    let mut collected = Vec::new();
    while let Ok(record) = recorded.try_recv() {
        if let Record::Event { event, .. } = record {
            collected.push(event);
        }
    }
    collected
}

fn stopped_with(log: &micro_testkit::EventLog) -> Option<String> {
    log.assistant_message_ends()
        .iter()
        .rev()
        .find_map(|assistant| assistant.error.clone())
}

#[tokio::test]
async fn a_turn_past_the_ceiling_ends_the_run_and_records_why() {
    let provider = two_turns();
    let tool = FakeTool::new("read").returning("contents");
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(tool.clone())],
        model(),
        "test-key",
    )
    .with_recorder(recorder)
    .with_budget(Budget::new(0.005, rates()));

    let (_, log) = run_agent(&mut agent, Message::user("read a.txt")).await;

    assert_eq!(provider.call_count(), 1, "no second request went out");
    assert_eq!(
        tool.call_count(),
        0,
        "and the tool it asked for did not run"
    );

    let stops: Vec<(f64, f64)> = events(&mut recorded)
        .iter()
        .filter_map(|event| match event {
            LedgerEvent::BudgetStop { limit, spent } => Some((*limit, *spent)),
            _ => None,
        })
        .collect();
    assert_eq!(stops.len(), 1, "recorded once");
    assert_eq!(stops[0].0, 0.005);
    assert!((stops[0].1 - 0.01).abs() < 1e-12, "what it had spent");

    let said = stopped_with(&log).expect("the run said why it stopped");
    assert!(said.contains("$0.0100 of its $0.0050 budget"), "{said}");
}

/// A run inside its ceiling is a run nothing happened to.
#[tokio::test]
async fn a_run_inside_its_ceiling_is_left_alone() {
    let provider = two_turns();
    let tool = FakeTool::new("read").returning("contents");
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(tool.clone())],
        model(),
        "test-key",
    )
    .with_recorder(recorder)
    .with_budget(Budget::new(1.0, rates()));

    let (_, log) = run_agent(&mut agent, Message::user("read a.txt")).await;

    assert_eq!(provider.call_count(), 2, "both turns went out");
    assert_eq!(tool.call_count(), 1);
    assert!(events(&mut recorded)
        .iter()
        .all(|event| !matches!(event, LedgerEvent::BudgetStop { .. })));
    assert_eq!(stopped_with(&log), None, "and nothing was reported wrong");
}

#[tokio::test]
async fn a_session_reopened_over_its_ceiling_still_answers_once() {
    let provider = two_turns();
    let tool = FakeTool::new("read").returning("contents");
    let (recorder, _recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(tool.clone())],
        model(),
        "test-key",
    )
    .with_recorder(recorder)
    .with_budget(Budget::new(0.005, rates()).already_spent(0.02));

    let (produced, _) = run_agent(&mut agent, Message::user("read a.txt")).await;

    assert_eq!(provider.call_count(), 1, "it answered once");
    assert!(
        produced
            .iter()
            .any(|message| matches!(message, Message::Assistant(assistant)
                if assistant.stop_reason == StopReason::ToolUse)),
        "and what it answered is in the conversation",
    );
}

#[tokio::test]
async fn a_run_with_no_ceiling_is_never_stopped() {
    let provider = two_turns();
    let tool = FakeTool::new("read").returning("contents");
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(tool.clone())],
        model(),
        "test-key",
    )
    .with_recorder(recorder);

    run_agent(&mut agent, Message::user("read a.txt")).await;

    assert_eq!(provider.call_count(), 2);
    assert!(events(&mut recorded)
        .iter()
        .all(|event| !matches!(event, LedgerEvent::BudgetStop { .. })));
}
