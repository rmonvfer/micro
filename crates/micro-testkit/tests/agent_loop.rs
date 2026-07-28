//! End-to-end tests of the agent loop, driven entirely against the testkit doubles.

use std::sync::Arc;

use micro_agent::Agent;
use micro_testkit::run_agent;
use micro_testkit::FakeProvider;
use micro_testkit::FakeTool;
use micro_testkit::Turn;
use micro_types::Message;
use micro_types::Model;
use micro_types::StopReason;
use micro_types::ThinkingLevel;
use serde_json::json;

fn model() -> Model {
    Model {
        id: "test-model".into(),
        provider: "fake".into(),
        base_url: "https://example.invalid".into(),
        max_tokens: 1024,
        thinking: ThinkingLevel::Off,
    }
}

fn agent(provider: &FakeProvider, tools: Vec<Arc<dyn micro_tools::Tool>>) -> Agent {
    Agent::new(Arc::new(provider.clone()), tools, model(), "test-key")
}

/// The text of a tool result message, for asserting on what the loop fed back.
fn tool_result_text(message: &Message) -> String {
    message
        .content()
        .iter()
        .map(micro_types::ContentBlock::as_text)
        .collect()
}

#[tokio::test]
async fn a_plain_text_turn_emits_its_events_in_order() {
    let provider = FakeProvider::once(Turn::streamed_text(["Hello", ", ", "world"]));
    let mut agent = agent(&provider, Vec::new());

    let (messages, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert_eq!(
        events.names(),
        vec![
            "AgentStart",
            // The prompt enters the conversation before the first request goes out.
            "MessageStart",
            "MessageEnd",
            "TurnStart",
            // The assistant message opens once the provider streams anything.
            "MessageStart",
            "MessageDelta", // Start
            "MessageDelta", // TextStart
            "MessageDelta", // TextDelta "Hello"
            "MessageDelta", // TextDelta ", "
            "MessageDelta", // TextDelta "world"
            "MessageDelta", // TextEnd
            "MessageEnd",
            "TurnEnd",
            "AgentEnd",
            "AgentSettled",
        ]
    );

    assert_eq!(events.streamed_text(), "Hello, world");
    assert_eq!(messages.len(), 2);
    assert_eq!(events.assistant_message_ends()[0].text(), "Hello, world");
    assert_eq!(provider.call_count(), 1);
}

#[tokio::test]
async fn the_first_request_carries_the_prompt_the_system_prompt_and_the_tools() {
    let provider = FakeProvider::once(Turn::text("done"));
    let read = FakeTool::new("read").with_description("read a file");
    let write = FakeTool::new("write");
    let mut agent =
        agent(&provider, vec![Arc::new(read), Arc::new(write)]).with_system_prompt("be brief");

    run_agent(&mut agent, Message::user("hi")).await;

    let request = provider.call(0);
    assert_eq!(request.system_prompt(), Some("be brief"));
    assert_eq!(request.message_roles(), vec!["user"]);
    assert_eq!(request.tool_names(), vec!["read", "write"]);
    assert_eq!(request.api_key, "test-key");
    assert_eq!(request.model.id, "test-model");
}

#[tokio::test]
async fn a_tool_call_is_executed_and_its_result_feeds_the_next_request() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("call_1", "read", json!({ "path": "a.txt" })))
        .turn(Turn::text("a.txt says hello"))
        .build();
    let read = FakeTool::new("read").returning("hello");
    let mut agent = agent(&provider, vec![Arc::new(read.clone())]);

    let (messages, events) = run_agent(&mut agent, Message::user("read a.txt")).await;

    // The tool ran exactly once, with the arguments the model supplied.
    assert_eq!(read.call_count(), 1);
    assert_eq!(read.call(0), json!({ "path": "a.txt" }));

    // Its result was appended as a tool result carrying the matching call id.
    let Some(Message::ToolResult {
        tool_call_id,
        tool_name,
        is_error,
        ..
    }) = messages.get(2)
    else {
        panic!(
            "expected a tool result at index 2, got {:?}",
            messages.get(2)
        );
    };
    assert_eq!(tool_call_id, "call_1");
    assert_eq!(tool_name, "read");
    assert!(!is_error);
    assert_eq!(tool_result_text(&messages[2]), "hello");

    // The loop then issued a second request whose context contains that result.
    assert_eq!(provider.call_count(), 2);
    let second = provider.call(1);
    assert_eq!(
        second.message_roles(),
        vec!["user", "assistant", "tool_result"]
    );
    assert_eq!(
        second.tool_results(),
        vec![("call_1", "read", "hello".to_string(), false)]
    );
    assert!(second.orphaned_tool_results().is_empty());
    assert!(second.unanswered_tool_calls().is_empty());

    assert_eq!(events.tool_ends(), vec![("call_1", "read", "hello", false)]);
}

#[tokio::test]
async fn several_tool_calls_in_one_message_all_run_in_order() {
    let provider = FakeProvider::builder()
        .turn(
            Turn::new()
                .with_tool_call("call_1", "read", json!({ "path": "a.txt" }))
                .with_tool_call("call_2", "read", json!({ "path": "b.txt" }))
                .with_tool_call("call_3", "read", json!({ "path": "c.txt" })),
        )
        .turn(Turn::text("read all three"))
        .build();
    let read = FakeTool::new("read").responses([
        Ok("alpha".to_string()),
        Ok("beta".to_string()),
        Ok("gamma".to_string()),
    ]);
    let mut agent = agent(&provider, vec![Arc::new(read.clone())]);

    let (_, events) = run_agent(&mut agent, Message::user("read them all")).await;

    assert_eq!(read.call_count(), 3);
    assert_eq!(
        read.calls(),
        vec![
            json!({ "path": "a.txt" }),
            json!({ "path": "b.txt" }),
            json!({ "path": "c.txt" }),
        ]
    );
    assert_eq!(
        events.tool_ends(),
        vec![
            ("call_1", "read", "alpha", false),
            ("call_2", "read", "beta", false),
            ("call_3", "read", "gamma", false),
        ]
    );

    // Every call is answered, in the order the model asked for them.
    let second = provider.call(1);
    let ids: Vec<&str> = second
        .tool_results()
        .into_iter()
        .map(|(id, ..)| id)
        .collect();
    assert_eq!(ids, vec!["call_1", "call_2", "call_3"]);
    assert!(second.unanswered_tool_calls().is_empty());
}

#[tokio::test]
async fn a_failing_tool_produces_an_error_result_and_the_loop_continues() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("call_1", "write", json!({ "path": "a.txt" })))
        .turn(Turn::text("the write failed, stopping"))
        .build();
    let write = FakeTool::new("write").failing("permission denied");
    let mut agent = agent(&provider, vec![Arc::new(write.clone())]);

    let (messages, events) = run_agent(&mut agent, Message::user("write a.txt")).await;

    assert_eq!(write.call_count(), 1);
    assert_eq!(
        events.tool_ends(),
        vec![("call_1", "write", "permission denied", true)]
    );

    let Some(Message::ToolResult { is_error, .. }) = messages.get(2) else {
        panic!("expected a tool result at index 2");
    };
    assert!(is_error, "a tool error must be flagged on the result");
    assert_eq!(tool_result_text(&messages[2]), "permission denied");

    // The failure is reported to the model rather than ending the run.
    assert_eq!(provider.call_count(), 2);
    assert_eq!(
        provider.call(1).tool_results(),
        vec![("call_1", "write", "permission denied".to_string(), true)]
    );
    assert_eq!(events.assistant_message_ends().len(), 2);
}

#[tokio::test]
async fn an_unknown_tool_produces_an_error_result_rather_than_a_panic() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("call_1", "teleport", json!({})))
        .turn(Turn::text("no such tool, understood"))
        .build();
    let read = FakeTool::new("read");
    let mut agent = agent(&provider, vec![Arc::new(read.clone())]);

    let (messages, events) = run_agent(&mut agent, Message::user("teleport")).await;

    assert_eq!(read.call_count(), 0);
    let (_, _, output, is_error) = events.tool_ends()[0];
    assert!(is_error);
    assert!(
        output.contains("teleport"),
        "the result should name the missing tool, got {output:?}"
    );

    // The loop keeps going and reports the failure back to the model.
    assert_eq!(provider.call_count(), 2);
    let (_, _, _, result_is_error) = provider.call(1).tool_results()[0];
    assert!(result_is_error);
    assert!(matches!(messages.get(2), Some(Message::ToolResult { .. })));
}

#[tokio::test]
async fn a_length_stop_fails_every_tool_call_without_executing_it() {
    let provider = FakeProvider::builder()
        .turn(
            Turn::new()
                .with_tool_call("call_1", "write", json!({ "path": "a.txt" }))
                .with_tool_call("call_2", "write", json!({ "path": "b.txt" }))
                .with_stop_reason(StopReason::Length),
        )
        .turn(Turn::text("re-issuing with complete arguments"))
        .build();
    let write = FakeTool::new("write");
    let mut agent = agent(&provider, vec![Arc::new(write.clone())]);

    let (messages, events) = run_agent(&mut agent, Message::user("write two files")).await;

    // The guard against silently truncated arguments: nothing ran.
    assert_eq!(
        write.call_count(),
        0,
        "a truncated response must not execute its tool calls"
    );

    let ends = events.tool_ends();
    assert_eq!(ends.len(), 2);
    for (_, _, output, is_error) in &ends {
        assert!(*is_error);
        assert!(
            output.contains("token limit"),
            "the result should explain why the call was skipped, got {output:?}"
        );
    }

    // Both calls are still answered, so the next request is well formed.
    let second = provider.call(1);
    assert!(second.unanswered_tool_calls().is_empty());
    assert!(second.tool_results().iter().all(|(.., is_error)| *is_error));
    assert_eq!(
        messages
            .iter()
            .filter(|m| matches!(m, Message::ToolResult { .. }))
            .count(),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn a_retryable_error_is_retried() {
    let provider = FakeProvider::builder()
        .turn(Turn::error("Fake returned 429: slow down"))
        .turn(Turn::text("recovered"))
        .build();
    let mut agent = agent(&provider, Vec::new());

    let (messages, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert_eq!(provider.call_count(), 2, "the request should be re-issued");
    assert_eq!(events.retries(), vec![(1, 1_000)]);
    assert_eq!(events.assistant_message_ends()[0].text(), "recovered");
    assert_eq!(
        events.assistant_message_ends()[0].stop_reason,
        StopReason::Stop
    );
    assert_eq!(messages.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_non_retryable_error_is_not_retried() {
    let provider = FakeProvider::builder()
        .turn(Turn::error("Fake returned 400: bad request"))
        .turn(Turn::text("never reached"))
        .build();
    let mut agent = agent(&provider, Vec::new());

    let (messages, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert_eq!(provider.call_count(), 1, "a 400 must not be re-issued");
    assert!(events.retries().is_empty());
    assert_eq!(provider.remaining_turns(), 1);

    let assistant = events.assistant_message_ends()[0];
    assert_eq!(assistant.stop_reason, StopReason::Error);
    assert_eq!(
        assistant.error.as_deref(),
        Some("Fake returned 400: bad request")
    );
    assert_eq!(messages.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn retries_stop_at_the_attempt_cap() {
    let provider = FakeProvider::builder()
        .turns((0..6).map(|_| Turn::error("Fake returned 503: overloaded")))
        .build();
    let mut agent = agent(&provider, Vec::new());

    let (_, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert_eq!(
        provider.call_count(),
        5,
        "five attempts, four of them retries"
    );
    assert_eq!(
        events.retries(),
        vec![(1, 1_000), (2, 2_000), (3, 4_000), (4, 8_000)],
        "backoff doubles between attempts"
    );
    assert_eq!(
        events.assistant_message_ends()[0].stop_reason,
        StopReason::Error
    );
}

#[tokio::test(start_paused = true)]
async fn a_stream_that_failed_after_emitting_text_is_not_retried() {
    // Content already shown to the user cannot be un-shown, so re-issuing an otherwise
    // retryable request would duplicate it.
    let provider = FakeProvider::builder()
        .turn(Turn::streamed_text(["partial"]).failing("Fake returned 429: slow down"))
        .turn(Turn::text("never reached"))
        .build();
    let mut agent = agent(&provider, Vec::new());

    let (_, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert_eq!(provider.call_count(), 1);
    assert!(events.retries().is_empty());
    assert_eq!(events.streamed_text(), "partial");
    assert_eq!(
        events.assistant_message_ends()[0].stop_reason,
        StopReason::Error
    );
}

#[tokio::test]
async fn the_loop_runs_as_many_turns_as_the_model_asks_for() {
    // Nothing in the loop caps the number of turns: it ends when the model stops
    // requesting tools, or when the provider fails. See the report on the runaway risk
    // this leaves open.
    let provider = FakeProvider::builder()
        .turns((0..4).map(|turn| {
            Turn::new().with_tool_call(format!("call_{turn}"), "read", json!({ "n": turn }))
        }))
        .build();
    let read = FakeTool::new("read").returning("more");
    let mut agent = agent(&provider, vec![Arc::new(read.clone())]);

    let (_, events) = run_agent(&mut agent, Message::user("keep going")).await;

    assert_eq!(read.call_count(), 4);
    // Four scripted turns plus the request that ran the script out.
    assert_eq!(provider.call_count(), 5);
    assert_eq!(
        events
            .assistant_message_ends()
            .last()
            .unwrap()
            .error
            .as_deref(),
        Some(FakeProvider::EXHAUSTED),
        "the run ended because the provider stopped answering, not because the loop stopped"
    );
}

#[tokio::test]
async fn the_returned_messages_match_the_ones_reported_by_agent_end() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("call_1", "read", json!({ "path": "a.txt" })))
        .turn(Turn::text("all done"))
        .build();
    let read = FakeTool::new("read").returning("contents");
    let mut agent = agent(&provider, vec![Arc::new(read)]);

    let (messages, events) = run_agent(&mut agent, Message::user("read a.txt")).await;

    assert_eq!(events.final_messages(), Some(messages.as_slice()));
    assert_eq!(
        messages.iter().map(role_of).collect::<Vec<_>>(),
        vec!["user", "assistant", "tool_result", "assistant"]
    );
    // The run's messages are also what the agent kept as the conversation.
    assert_eq!(agent.messages(), messages.as_slice());
}

#[tokio::test]
async fn a_turn_that_only_emits_done_still_reports_message_end() {
    // A provider is free to answer with nothing but the terminal event. The loop must
    // still close the assistant message. Note it emits no assistant `MessageStart` here,
    // because that is triggered by the first streamed event rather than by the turn
    // beginning — see the report on that asymmetry.
    let provider = FakeProvider::once(Turn::text("instant").without_deltas());
    let mut agent = agent(&provider, Vec::new());

    let (messages, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert!(events.deltas().is_empty());
    assert_eq!(events.assistant_message_ends().len(), 1);
    assert_eq!(events.assistant_message_ends()[0].text(), "instant");
    assert_eq!(messages.len(), 2);
    // Settling is the last thing a run says, after the run itself is reported over.
    assert_eq!(events.names().last(), Some(&"AgentSettled"));
}

/// Documents a defect in `micro-agent`: a retried request emits a second assistant
/// `MessageStart` without an intervening `MessageEnd`, so a consumer that opens a message
/// bubble on `MessageStart` opens two for one response.
///
/// `stream_once` resets its `started` flag on every attempt, so any attempt that streams a
/// non-content event (here `StreamEvent::Start`) before failing emits `MessageStart`
/// again. Ignored until `micro-agent` tracks `started` across attempts.
#[tokio::test(start_paused = true)]

async fn a_retry_does_not_reopen_the_assistant_message() {
    let provider = FakeProvider::builder()
        .turn(Turn::error("Fake returned 429: slow down").with_start(true))
        .turn(Turn::text("recovered"))
        .build();
    let mut agent = agent(&provider, Vec::new());

    let (_, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert_eq!(events.retries().len(), 1);
    // One for the prompt, one for the assistant response.
    assert_eq!(
        events.count("MessageStart"),
        2,
        "a retried response must not reopen its assistant message"
    );
    assert_eq!(events.count("MessageEnd"), 2);
}

fn role_of(message: &Message) -> &'static str {
    match message {
        Message::User { .. } => "user",
        Message::Assistant(_) => "assistant",
        Message::ToolResult { .. } => "tool_result",
    }
}
