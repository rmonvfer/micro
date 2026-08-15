//! What every request opens with, and the only ways it is allowed to change.

use micro_agent::Agent;
use micro_agent::Hooks;
use micro_agent::PrefixControl;
use micro_agent::Record;
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
use serde_json::json;
use std::sync::Arc;

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

fn span(text: &str, source: EventSource) -> PrefixSpan {
    PrefixSpan {
        source,
        bytes: text.len() as u64,
        hash: content_hash(text.as_bytes()),
    }
}

fn drain(recorded: &mut tokio::sync::mpsc::UnboundedReceiver<Record>) -> Vec<LedgerEvent> {
    let mut events = Vec::new();
    while let Ok(record) = recorded.try_recv() {
        if let Record::Event { event, .. } = record {
            events.push(event);
        }
    }
    events
}

fn prefix_changes(events: &[LedgerEvent]) -> Vec<(String, String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            LedgerEvent::PrefixChanged {
                reason,
                from_hash,
                to_hash,
            } => Some((reason.clone(), from_hash.clone(), to_hash.clone())),
            _ => None,
        })
        .collect()
}

fn prefix_hashes(events: &[LedgerEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            LedgerEvent::TurnRequest { prefix_hash, .. } => Some(prefix_hash.clone()),
            _ => None,
        })
        .collect()
}

/// Two turns of one run send the same prefix, byte for byte, and the session says so: the
/// hash it records is the same one twice, and nothing claims anything changed.
#[tokio::test]
async fn consecutive_turns_open_with_the_same_prefix() {
    let provider = FakeProvider::builder()
        .turn(Turn::text("looking").with_tool_call("call_1", "read", json!({ "path": "a.txt" })))
        .turn(Turn::text("done"))
        .build();
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(FakeTool::new("read").returning("contents"))],
        model(),
        "test-key",
    )
    .with_system_prompt("be brief")
    .with_prefix_spans(vec![span("be brief", EventSource::SystemPrompt)])
    .with_recorder(recorder);

    run_agent(&mut agent, Message::user("read a.txt")).await;
    let events = drain(&mut recorded);

    let hashes = prefix_hashes(&events);
    assert_eq!(hashes.len(), 2, "one request per turn");
    assert_eq!(hashes[0], hashes[1], "and the same head on both");
    assert!(prefix_changes(&events).is_empty(), "nothing moved");

    // What the provider was actually handed agrees with what was recorded about it.
    assert_eq!(provider.call(0).system_prompt(), Some("be brief"));
    assert_eq!(provider.call(1).system_prompt(), Some("be brief"));
    assert_eq!(provider.call(0).tool_names(), provider.call(1).tool_names());
}

/// Re-reading the project's instructions reaches the model at the next turn, and reaches
/// the ledger with the reason it happened. The request already in flight keeps the prompt
/// it was built with.
#[tokio::test]
async fn a_reloaded_prompt_lands_at_the_next_turn_and_is_recorded_once() {
    let provider = FakeProvider::builder()
        .turn(Turn::text("looking").with_tool_call("call_1", "read", json!({})))
        .turn(Turn::text("done"))
        .build();
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(FakeTool::new("read").returning("contents"))],
        model(),
        "test-key",
    )
    .with_system_prompt("be brief")
    .with_prefix_spans(vec![span("be brief", EventSource::SystemPrompt)])
    .with_recorder(recorder);

    let prefix = agent.prefix_control();
    run_agent(&mut agent, Message::user("first")).await;
    let first = drain(&mut recorded);
    assert!(prefix_changes(&first).is_empty());

    prefix.change(
        "be brief\n\nthe project says: run the tests",
        vec![
            span("be brief", EventSource::SystemPrompt),
            span(
                "\n\nthe project says: run the tests",
                EventSource::ProjectInstructions,
            ),
        ],
        "reload",
    );
    // Read back before any turn runs: whoever asked for the change is answered with what
    // they asked for, not with what the last request happened to carry.
    assert!(prefix.system_prompt().contains("run the tests"));

    run_agent(&mut agent, Message::user("second")).await;
    let second = drain(&mut recorded);

    let changes = prefix_changes(&second);
    assert_eq!(changes.len(), 1, "recorded once, not once a request");
    assert_eq!(changes[0].0, "reload");
    assert_ne!(changes[0].1, changes[0].2, "and the head really moved");

    assert_eq!(
        provider.call(1).system_prompt(),
        Some("be brief"),
        "the turn that was already running kept its prompt"
    );
    assert_eq!(
        provider.call(2).system_prompt(),
        Some("be brief\n\nthe project says: run the tests"),
    );

    // The recorded spans describe the prompt that was sent, so a reader can say which part
    // of it moved rather than only that something did.
    let sources: Vec<EventSource> = second
        .iter()
        .find_map(|event| match event {
            LedgerEvent::TurnRequest { prefix_spans, .. } => Some(prefix_spans.clone()),
            _ => None,
        })
        .expect("the second run recorded a request")
        .into_iter()
        .map(|span| span.source)
        .collect();
    assert_eq!(
        sources,
        vec![EventSource::SystemPrompt, EventSource::ProjectInstructions]
    );
}

/// Narrowing the tools is a change to the head of the request, so it waits for a boundary
/// and is recorded when it lands — rather than taking effect silently in the middle of a
/// run, which is what reading the list on every request amounted to.
#[tokio::test]
async fn narrowing_the_tools_lands_at_a_turn_boundary_and_is_recorded() {
    let provider = FakeProvider::builder()
        .turn(Turn::text("looking").with_tool_call("call_1", "read", json!({})))
        .turn(Turn::text("done"))
        .build();
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();
    let offered: Arc<std::sync::RwLock<Option<Vec<String>>>> = Arc::default();

    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![
            Arc::new(FakeTool::new("read").returning("contents")),
            Arc::new(FakeTool::new("write").returning("written")),
        ],
        model(),
        "test-key",
    )
    .with_offered_tools(Arc::clone(&offered))
    .with_system_prompt("be brief")
    .with_recorder(recorder);

    run_agent(&mut agent, Message::user("first")).await;
    drain(&mut recorded);
    assert_eq!(provider.call(0).tool_names(), vec!["read", "write"]);

    *offered.write().unwrap() = Some(vec!["read".to_string()]);
    run_agent(&mut agent, Message::user("second")).await;
    let events = drain(&mut recorded);

    let changes = prefix_changes(&events);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].0, "tools");
    assert_eq!(provider.call(2).tool_names(), vec!["read"]);
}

/// A change asked for while a turn is running does not reach that turn. Anything else
/// would rewrite a request the ledger had already described.
#[tokio::test]
async fn a_change_asked_for_mid_turn_waits_for_the_next_one() {
    struct AsksMidTurn {
        prefix: PrefixControl,
    }

    #[async_trait::async_trait]
    impl Hooks for AsksMidTurn {
        async fn before_request(&self, context: micro_types::Context) -> micro_types::Context {
            self.prefix
                .change("something else entirely", Vec::new(), "extension");
            context
        }
    }

    let provider = FakeProvider::builder()
        .turn(Turn::text("looking").with_tool_call("call_1", "read", json!({})))
        .turn(Turn::text("done"))
        .build();
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(FakeTool::new("read").returning("contents"))],
        model(),
        "test-key",
    )
    .with_system_prompt("be brief")
    .with_recorder(recorder);
    let prefix = agent.prefix_control();
    let mut agent = agent.with_hooks(Arc::new(AsksMidTurn { prefix }));

    run_agent(&mut agent, Message::user("go")).await;
    let events = drain(&mut recorded);

    assert_eq!(
        provider.call(0).system_prompt(),
        Some("be brief"),
        "the request that was being assembled is not the one that changed"
    );
    assert_eq!(
        provider.call(1).system_prompt(),
        Some("something else entirely"),
        "the next turn is"
    );
    let changes = prefix_changes(&events);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].0, "extension");
}

/// A conversation that arrives holding an unanswered tool call is repaired where it
/// arrives, so the repair is not still being made between two turns of a live session —
/// which would move the middle of a history a provider had already cached.
#[tokio::test]
async fn a_history_is_repaired_where_it_is_installed_and_not_between_turns() {
    let abandoned = vec![
        Message::user("read a.txt"),
        Message::Assistant(micro_types::AssistantMessage {
            content: vec![micro_types::ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "read".into(),
                arguments: json!({}),
                signature: None,
            }],
            provider: "fake".into(),
            model: "test-model".into(),
            usage: Default::default(),
            stop_reason: micro_types::StopReason::ToolUse,
            error: None,
            timestamp: 0,
        }),
        Message::user("never mind, carry on"),
    ];

    let provider = FakeProvider::builder()
        .turn(Turn::text("first"))
        .turn(Turn::text("second"))
        .build();
    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(FakeTool::new("read").returning("contents"))],
        model(),
        "test-key",
    )
    .with_history(abandoned);

    run_agent(&mut agent, Message::user("first")).await;
    let first_call = provider.call(0);
    let repaired = first_call.messages();
    assert!(
        matches!(repaired[2], Message::ToolResult { .. }),
        "the abandoned call was answered beside the call it belongs to"
    );

    run_agent(&mut agent, Message::user("second")).await;
    let second_call = provider.call(1);
    let second = second_call.messages();

    // The second request opens with exactly the first one's messages: nothing was inserted
    // into the middle of the conversation between the two turns.
    assert_eq!(
        second[..repaired.len()],
        repaired[..],
        "the conversation only grew at the end"
    );
}
