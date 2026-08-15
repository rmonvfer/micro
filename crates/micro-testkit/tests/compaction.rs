//! End-to-end tests of compaction inside the agent loop.

use std::sync::Arc;

use micro_agent::Agent;
use micro_agent::ProviderSummarizer;
use micro_context::estimate_context_tokens;
use micro_context::is_self_contained;
use micro_context::is_summary;
use micro_context::summary_text;
use micro_context::CompactionConfig;
use micro_context::Summarizer;
use micro_testkit::run_agent;
use micro_testkit::FakeProvider;
use micro_testkit::FakeSummarizer;
use micro_testkit::FakeTool;
use micro_testkit::Turn;
use micro_types::AssistantMessage;
use micro_types::ContentBlock;
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

fn assistant(text: &str) -> Message {
    Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::text(text)],
        provider: "fake".into(),
        model: "test-model".into(),
        usage: Usage::default(),
        stop_reason: StopReason::Stop,
        error: None,
        timestamp: 0,
    })
}

fn tool_calls(ids: &[&str]) -> Message {
    Message::Assistant(AssistantMessage {
        content: ids
            .iter()
            .map(|id| ContentBlock::ToolCall {
                id: (*id).into(),
                name: "read".into(),
                arguments: json!({ "path": "a.txt" }),

                signature: None,
            })
            .collect(),
        provider: "fake".into(),
        model: "test-model".into(),
        usage: Usage::default(),
        stop_reason: StopReason::ToolUse,
        error: None,
        timestamp: 0,
    })
}

/// One exchange: a prompt, a tool call, its result, and a reply, each padded so the conversation
/// reaches a predictable size.
fn turn(index: usize, padding: usize) -> Vec<Message> {
    let id = format!("call_{index}");
    vec![
        Message::user("u".repeat(padding)),
        tool_calls(&[&id]),
        Message::tool_result(&id, "read", "r".repeat(padding), false),
        assistant(&"a".repeat(padding)),
    ]
}

fn conversation(turns: usize, padding: usize) -> Vec<Message> {
    (0..turns).flat_map(|index| turn(index, padding)).collect()
}

/// An exchange whose tool calls all go out at once, which is the shape a cut is most likely to
/// split.
fn parallel_turn(index: usize, width: usize, padding: usize) -> Vec<Message> {
    let ids: Vec<String> = (0..width).map(|n| format!("call_{index}_{n}")).collect();
    let borrowed: Vec<&str> = ids.iter().map(String::as_str).collect();

    let mut messages = vec![Message::user("u".repeat(padding)), tool_calls(&borrowed)];
    messages.extend(
        ids.iter()
            .map(|id| Message::tool_result(id, "read", "r".repeat(padding), false)),
    );
    messages.push(assistant(&"a".repeat(padding)));
    messages
}

#[tokio::test]
async fn a_conversation_over_the_window_is_compacted_before_the_next_request() {
    let history = conversation(10, 400);
    let provider = FakeProvider::once(Turn::text("carrying on"));
    let summarizer = FakeSummarizer::new("what happened earlier");

    let mut agent = Agent::new(Arc::new(provider.clone()), Vec::new(), model(), "test-key")
        .with_history(history.clone())
        .with_context_window(2_000)
        .with_summarizer(Arc::new(summarizer.clone()));

    run_agent(&mut agent, Message::user("continue")).await;

    assert_eq!(summarizer.call_count(), 1, "the summarizer should have run");

    let sent = provider.call(0).context.messages;
    assert!(
        sent.len() < history.len() + 1,
        "the context should be shorter"
    );
    assert_eq!(summary_text(&sent[0]), Some("what happened earlier"));
    assert!(
        estimate_context_tokens(&sent) < estimate_context_tokens(&history),
        "compaction must reclaim tokens"
    );
}

#[tokio::test]
async fn a_conversation_that_fits_is_left_alone() {
    let provider = FakeProvider::once(Turn::text("fine"));
    let summarizer = FakeSummarizer::new("never used");

    let mut agent = Agent::new(Arc::new(provider.clone()), Vec::new(), model(), "test-key")
        .with_history(conversation(2, 20))
        .with_context_window(1_000_000)
        .with_summarizer(Arc::new(summarizer.clone()));

    run_agent(&mut agent, Message::user("hi")).await;

    assert_eq!(summarizer.call_count(), 0);
    assert!(!provider.call(0).context.messages.iter().any(is_summary));
}

#[tokio::test]
async fn compaction_can_be_turned_off() {
    let history = conversation(10, 400);
    let provider = FakeProvider::once(Turn::text("carrying on"));
    let summarizer = FakeSummarizer::new("unused");

    let mut agent = Agent::new(Arc::new(provider.clone()), Vec::new(), model(), "test-key")
        .with_history(history.clone())
        .with_context_window(2_000)
        .with_summarizer(Arc::new(summarizer.clone()))
        .without_compaction();

    run_agent(&mut agent, Message::user("continue")).await;

    assert_eq!(summarizer.call_count(), 0);
    assert_eq!(provider.call(0).context.messages.len(), history.len() + 1);
}

#[tokio::test]
async fn the_most_recent_messages_survive_verbatim() {
    let history = conversation(10, 400);
    let provider = FakeProvider::once(Turn::text("carrying on"));
    let summarizer = FakeSummarizer::new("earlier work");
    let prompt = Message::user("continue");

    let mut agent = Agent::new(Arc::new(provider.clone()), Vec::new(), model(), "test-key")
        .with_history(history.clone())
        .with_context_window(2_000)
        .with_summarizer(Arc::new(summarizer.clone()));

    run_agent(&mut agent, prompt.clone()).await;

    let sent = provider.call(0).context.messages;
    
    let kept = &sent[1..];
    let mut original = history;
    original.push(prompt);
    assert_eq!(kept, &original[original.len() - kept.len()..]);

    
    let summarized = summarizer.call(0);
    assert_eq!(summarized.len() + kept.len(), original.len());
    assert_eq!(summarized, original[..summarized.len()]);
}

#[tokio::test]
async fn a_tool_call_and_its_result_are_never_split_by_the_cut() {
    
    for width in [1, 3, 5] {
        let history: Vec<Message> = (0..8)
            .flat_map(|index| parallel_turn(index, width, 300))
            .collect();
        let mut compacted = 0;
        let mut untouched = 0;

        for window in (600..=6_000).step_by(200) {
            let provider = FakeProvider::once(Turn::text("ok"));
            let summarizer = FakeSummarizer::new("earlier work");

            let mut agent = Agent::new(Arc::new(provider.clone()), Vec::new(), model(), "test-key")
                .with_history(history.clone())
                .with_context_window(window)
                .with_summarizer(Arc::new(summarizer.clone()));

            run_agent(&mut agent, Message::user("continue")).await;

            let sent = provider.call(0).context.messages;
            assert!(
                is_self_contained(&sent),
                "width {width} window {window}: the request orphaned a tool result"
            );

            match summarizer.calls().first() {
                
                Some(summarized) => {
                    assert!(
                        is_self_contained(summarized),
                        "width {width} window {window}: the summarized half orphaned a tool result"
                    );
                    compacted += 1;
                }
                None => untouched += 1,
            }
        }

        
        assert!(compacted > 0, "width {width}: nothing was ever compacted");
        assert!(untouched > 0, "width {width}: everything was compacted");
    }
}

#[tokio::test]
async fn a_tool_result_produced_this_run_is_never_split_from_its_call() {
    
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("call_live", "read", json!({ "path": "a.txt" })))
        .turn(Turn::text("done"))
        .build();
    let summarizer = FakeSummarizer::new("earlier work");
    let read = FakeTool::new("read").returning("x".repeat(4_000));

    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        vec![Arc::new(read)],
        model(),
        "test-key",
    )
    .with_history(conversation(6, 400))
    .with_context_window(2_000)
    .with_summarizer(Arc::new(summarizer.clone()));

    run_agent(&mut agent, Message::user("read a.txt")).await;

    let second = provider.call(1);
    assert!(is_self_contained(&second.context.messages));
    assert!(second.unanswered_tool_calls().is_empty());
    assert!(second.orphaned_tool_results().is_empty());
    assert!(
        second
            .context
            .messages
            .iter()
            .any(|message| matches!(message, Message::ToolResult { tool_call_id, .. } if tool_call_id == "call_live")),
        "the result produced this run should still be in context"
    );
}

/// Summarizing is a request like any other: it is made on behalf of this conversation.
#[tokio::test]
async fn what_summarizing_cost_is_recorded_with_the_compaction_it_paid_for() {
    let provider = FakeProvider::builder()
        .turn(Turn::text("## Goal\nship it").with_usage(Usage {
            input: 900,
            output: 120,
            cache_read: 0,
            cache_write: 0,
        }))
        .build();
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(Arc::new(provider.clone()), Vec::new(), model(), "test-key")
        .with_history(conversation(10, 400))
        .with_context_window(2_000)
        .with_recorder(recorder)
        .with_cache_key("session-7");

    agent.compact_now().await.expect("a summary");

    assert_eq!(
        provider.call(0).context.cache_key.as_deref(),
        Some("session-7"),
        "the summarizer asks as part of the conversation it is summarizing"
    );

    let compaction = std::iter::from_fn(|| recorded.try_recv().ok())
        .find_map(|record| match record {
            micro_agent::Record::Compacted { cost, .. } => Some(cost),
            _ => None,
        })
        .expect("the compaction was recorded");
    assert_eq!(compaction.usage.input, 900);
    assert_eq!(compaction.usage.output, 120);
    assert_eq!(compaction.model, "test-model");
}

#[tokio::test]
async fn the_summary_is_reported_so_a_renderer_can_show_it() {
    let provider = FakeProvider::once(Turn::text("carrying on"));
    let summarizer = FakeSummarizer::new("what happened earlier");

    let mut agent = Agent::new(Arc::new(provider), Vec::new(), model(), "test-key")
        .with_history(conversation(10, 400))
        .with_context_window(2_000)
        .with_summarizer(Arc::new(summarizer));

    let (_, events) = run_agent(&mut agent, Message::user("continue")).await;

    let announced: Vec<&Message> = events
        .message_ends()
        .into_iter()
        .filter(|message| is_summary(message))
        .collect();
    assert_eq!(announced.len(), 1);
    assert_eq!(summary_text(announced[0]), Some("what happened earlier"));

    
    let summary_at = events
        .events()
        .iter()
        .position(|event| {
            matches!(event, micro_types::AgentEvent::MessageStart { message } if is_summary(message))
        })
        .expect("the summary should be announced");
    assert!(summary_at > events.position("TurnStart").unwrap());
    assert!(summary_at < events.position("MessageDelta").unwrap());
}

#[tokio::test]
async fn compaction_is_recorded_rather_than_only_applied() {
    
    let history = conversation(10, 400);
    let provider = FakeProvider::once(Turn::text("carrying on"));
    let summarizer = FakeSummarizer::new("earlier work");
    let (recorder, mut recorded) = tokio::sync::mpsc::unbounded_channel();

    let mut agent = Agent::new(Arc::new(provider.clone()), Vec::new(), model(), "test-key")
        .with_history(history)
        .with_context_window(2_000)
        .with_summarizer(Arc::new(summarizer))
        .with_recorder(recorder);

    let (messages, events) = run_agent(&mut agent, Message::user("continue")).await;

    
    assert_eq!(messages.len(), 2);
    assert!(!messages.iter().any(is_summary));
    assert_eq!(events.final_messages(), Some(messages.as_slice()));

    let mut persisted = Vec::new();
    while let Ok(record) = recorded.try_recv() {
        persisted.push(record);
    }

    let written: Vec<micro_types::Message> = persisted
        .iter()
        .filter_map(|record| match record {
            micro_agent::Record::Message(message) => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(written, messages, "every message still reaches the log");

    let compactions: Vec<&micro_agent::Record> = persisted
        .iter()
        .filter(|record| matches!(record, micro_agent::Record::Compacted { .. }))
        .collect();
    assert_eq!(compactions.len(), 1, "and the compaction is recorded once");
    let micro_agent::Record::Compacted { summary, kept, .. } = compactions[0] else {
        unreachable!("filtered above")
    };
    assert_eq!(summary, "earlier work");
    assert!(*kept > 0, "it says how much of the conversation it kept");
}

#[tokio::test]
async fn a_failing_summarizer_leaves_the_conversation_intact() {
    let history = conversation(10, 400);
    let provider = FakeProvider::once(Turn::text("carrying on"));
    let summarizer = FakeSummarizer::failing("the model said no");

    let mut agent = Agent::new(Arc::new(provider.clone()), Vec::new(), model(), "test-key")
        .with_history(history.clone())
        .with_context_window(2_000)
        .with_summarizer(Arc::new(summarizer.clone()));

    let (messages, events) = run_agent(&mut agent, Message::user("continue")).await;

    assert_eq!(summarizer.call_count(), 1);
    
    assert_eq!(provider.call_count(), 1);
    assert_eq!(provider.call(0).context.messages.len(), history.len() + 1);
    assert!(!provider.call(0).context.messages.iter().any(is_summary));
    assert_eq!(events.assistant_message_ends()[0].text(), "carrying on");
    assert_eq!(messages.len(), 2);
}

#[tokio::test]
async fn the_trigger_fraction_is_honoured() {
    let history = conversation(10, 400);
    let tokens = estimate_context_tokens(&history);

    
    let window = tokens * 4;
    for (config, expected) in [
        (CompactionConfig::new(0.1, 0.05).unwrap(), 1),
        (CompactionConfig::new(0.9, 0.3).unwrap(), 0),
    ] {
        let provider = FakeProvider::once(Turn::text("ok"));
        let summarizer = FakeSummarizer::new("earlier work");

        let mut agent = Agent::new(Arc::new(provider), Vec::new(), model(), "test-key")
            .with_history(history.clone())
            .with_context_window(window)
            .with_summarizer(Arc::new(summarizer.clone()))
            .with_compaction(config);

        run_agent(&mut agent, Message::user("continue")).await;

        assert_eq!(summarizer.call_count(), expected, "for {config:?}");
    }
}

#[tokio::test]
async fn the_provider_summarizer_asks_the_model_for_a_summary() {
    let provider = FakeProvider::once(Turn::text("## Goal\nship the thing"));
    let summarizer = ProviderSummarizer::new(Arc::new(provider.clone()), model(), "test-key");

    let summary = summarizer
        .summarize(&[Message::user("find the bug"), assistant("found it")])
        .await
        .unwrap();

    assert_eq!(summary.text, "## Goal\nship the thing");

    
    let request = provider.call(0);
    assert!(request.tool_names().is_empty());
    assert_eq!(request.message_roles(), vec!["user"]);
    assert_eq!(request.api_key, "test-key");

    let asked: String = request.context.messages[0]
        .content()
        .iter()
        .map(ContentBlock::as_text)
        .collect();
    assert!(asked.contains("find the bug"));
    assert!(asked.contains("found it"));
    assert!(asked.contains(micro_context::COMPACTION_PROMPT));
}

#[tokio::test]
async fn the_provider_summarizer_does_not_pay_for_thinking() {
    let provider = FakeProvider::once(Turn::text("a summary"));
    let mut thinking_model = model();
    thinking_model.thinking = ThinkingLevel::High;
    let summarizer = ProviderSummarizer::new(Arc::new(provider.clone()), thinking_model, "k");

    summarizer.summarize(&[Message::user("hi")]).await.unwrap();

    assert_eq!(provider.call(0).model.thinking, ThinkingLevel::Off);
}

#[tokio::test]
async fn a_provider_failure_fails_summarization() {
    let provider = FakeProvider::once(Turn::error("Fake returned 500: boom"));
    let summarizer = ProviderSummarizer::new(Arc::new(provider), model(), "test-key");

    let error = summarizer
        .summarize(&[Message::user("hi")])
        .await
        .unwrap_err();

    assert!(error.to_string().contains("boom"));
}

#[tokio::test]
async fn an_empty_summary_is_rejected() {
    
    let provider = FakeProvider::once(Turn::text("   "));
    let summarizer = ProviderSummarizer::new(Arc::new(provider), model(), "test-key");

    assert!(summarizer.summarize(&[Message::user("hi")]).await.is_err());
}

#[tokio::test]
async fn the_agent_summarizes_with_its_own_provider_by_default() {
    
    let provider = FakeProvider::builder()
        .turn(Turn::text("## Goal\nkeep going"))
        .turn(Turn::text("carrying on"))
        .build();

    let mut agent = Agent::new(Arc::new(provider.clone()), Vec::new(), model(), "test-key")
        .with_history(conversation(10, 400))
        .with_context_window(2_000);

    run_agent(&mut agent, Message::user("continue")).await;

    assert_eq!(provider.call_count(), 2);
    assert!(provider.call(0).tool_names().is_empty());
    assert_eq!(
        summary_text(&provider.call(1).context.messages[0]),
        Some("## Goal\nkeep going")
    );
}
