//! What the loop records about a run beyond the conversation.

use std::sync::Arc;

use micro_agent::Agent;
use micro_agent::Hooks;
use micro_agent::Record;
use micro_agent::ToolDecision;
use micro_provider::Provider;
use micro_testkit::run_agent;
use micro_testkit::FakeProvider;
use micro_testkit::FakeTool;
use micro_testkit::Turn;
use micro_types::content_hash;
use micro_types::EventSource;
use micro_types::LedgerEvent;
use micro_types::Message;
use micro_types::Model;
use micro_types::PrefixSpan;
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

/// Everything the run handed the recorder, in the order it produced it.
fn drain(recorded: &mut tokio::sync::mpsc::UnboundedReceiver<Record>) -> Vec<Record> {
    let mut records = Vec::new();
    while let Ok(record) = recorded.try_recv() {
        records.push(record);
    }
    records
}

fn events(records: &[Record]) -> Vec<LedgerEvent> {
    records
        .iter()
        .filter_map(|record| match record {
            Record::Event { event, .. } => Some(event.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn every_turn_records_the_request_it_issued_and_what_it_cost() {
    let provider = FakeProvider::builder()
        .turn(
            Turn::text("looking")
                .with_tool_call("call_1", "read", json!({ "path": "a.txt" }))
                .with_usage(Usage {
                    input: 10,
                    output: 2,
                    cache_read: 0,
                    cache_write: 0,
                }),
        )
        .turn(Turn::text("done").with_usage(Usage {
            input: 20,
            output: 4,
            cache_read: 8,
            cache_write: 0,
        }))
        .build();
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(FakeTool::new("read").returning("contents"))],
        model(),
        "test-key",
    )
    .with_system_prompt("you are micro")
    .with_prefix_spans(vec![PrefixSpan {
        source: EventSource::SystemPrompt,
        bytes: 13,
        hash: content_hash(b"you are micro"),
    }])
    .with_recorder(recorder);

    run_agent(&mut agent, Message::user("read a.txt")).await;
    let records = drain(&mut recorded);
    let recorded_events = events(&records);

    let turns: Vec<(u64, u32)> = recorded_events
        .iter()
        .filter_map(|event| match event {
            LedgerEvent::TurnRequest { turn, attempt, .. } => Some((*turn, *attempt)),
            _ => None,
        })
        .collect();
    assert_eq!(
        turns,
        vec![(1, 1), (2, 1)],
        "one request per turn, in order"
    );

    let billed: Vec<(u64, Usage)> = recorded_events
        .iter()
        .filter_map(|event| match event {
            LedgerEvent::TurnUsage { turn, usage, .. } => Some((*turn, *usage)),
            _ => None,
        })
        .collect();
    assert_eq!(billed.len(), 2);
    assert_eq!(billed[0].0, 1);
    assert_eq!(billed[1].1.cache_read, 8, "what the provider reported");

    let LedgerEvent::TurnRequest {
        request_hash,
        prefix_spans,
        ..
    } = &recorded_events
        .iter()
        .find(|event| matches!(event, LedgerEvent::TurnRequest { turn: 2, .. }))
        .expect("the second turn")
    else {
        unreachable!("matched above")
    };
    let sent = provider.call(1);
    let body = serde_json::to_vec(&provider.payload(&sent.model, &sent.context)).unwrap();
    assert_eq!(*request_hash, content_hash(&body));
    assert_eq!(
        prefix_spans.len(),
        1,
        "the prompt is attributed as it was built"
    );
}

#[tokio::test]
async fn content_a_record_names_is_handed_over_once() {
    let provider = FakeProvider::builder()
        .turn(Turn::text("looking").with_tool_call("call_1", "read", json!({})))
        .turn(Turn::text("done"))
        .build();
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(
        Arc::new(provider),
        vec![Arc::new(FakeTool::new("read").returning("contents"))],
        model(),
        "test-key",
    )
    .with_system_prompt("you are micro")
    .with_recorder(recorder);

    run_agent(&mut agent, Message::user("read a.txt")).await;
    let records = drain(&mut recorded);

    let carried: Vec<String> = records
        .iter()
        .filter_map(|record| match record {
            Record::Event { blobs, .. } => Some(blobs.clone()),
            _ => None,
        })
        .flatten()
        .map(|(hash, _)| hash)
        .collect();
    let mut once = carried.clone();
    once.sort();
    once.dedup();
    assert_eq!(carried.len(), once.len(), "nothing was carried twice");

    let turns = records
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Event {
                    event: LedgerEvent::TurnRequest { .. },
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        turns, 2,
        "one request opened the tool call and one closed it"
    );

    assert_eq!(
        carried.len(),
        3 + turns,
        "the prompt, the tool definitions and the model, then one body per turn: {carried:?}"
    );
    assert!(carried.contains(&content_hash(b"you are micro")));
}

/// A refused tool call reaches the model as a failed call, which says nothing about why.
#[tokio::test]
async fn a_refused_call_is_recorded_as_a_refusal() {
    struct Refusing;

    #[async_trait::async_trait]
    impl Hooks for Refusing {
        async fn before_tool(
            &self,
            _id: &str,
            _name: &str,
            _arguments: &serde_json::Value,
        ) -> ToolDecision {
            ToolDecision::Refuse("not while the deploy is running".into())
        }
    }

    let provider = FakeProvider::builder()
        .turn(Turn::text("looking").with_tool_call("call_1", "read", json!({})))
        .turn(Turn::text("all right then"))
        .build();
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(
        Arc::new(provider),
        vec![Arc::new(FakeTool::new("read").returning("contents"))],
        model(),
        "test-key",
    )
    .with_hooks(Arc::new(Refusing))
    .with_recorder(recorder);

    run_agent(&mut agent, Message::user("read a.txt")).await;
    let records = drain(&mut recorded);

    let denials: Vec<LedgerEvent> = events(&records)
        .into_iter()
        .filter(|event| matches!(event, LedgerEvent::ToolDenied { .. }))
        .collect();
    assert_eq!(
        denials,
        vec![LedgerEvent::ToolDenied {
            tool: "read".into(),
            reason: "not while the deploy is running".into(),
            source: EventSource::Extension(String::new()),
        }]
    );
}
