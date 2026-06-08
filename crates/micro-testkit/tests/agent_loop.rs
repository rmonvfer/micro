//! End-to-end tests of the agent loop, driven entirely against the testkit doubles.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::time::Duration;

use micro_agent::Agent;
use micro_provider::Provider;
use micro_testkit::run_agent;
use micro_testkit::FakeProvider;
use micro_testkit::FakeTool;
use micro_testkit::Turn;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::StopReason;
use micro_types::StreamEvent;
use micro_types::ThinkingLevel;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

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
            "MessageStart",
            "MessageEnd",
            "TurnStart",
            "MessageStart",
            "MessageDelta",
            "MessageDelta",
            "MessageDelta",
            "MessageDelta",
            "MessageDelta",
            "MessageDelta",
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

    assert_eq!(read.call_count(), 1);
    assert_eq!(read.call(0), json!({ "path": "a.txt" }));

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
async fn a_tool_removed_from_the_offered_set_cannot_be_executed() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("call_1", "write", json!({ "path": "a.txt" })))
        .turn(Turn::text("done"))
        .build();
    let read = FakeTool::new("read");
    let write = FakeTool::new("write");
    let offered = Arc::new(RwLock::new(Some(vec![
        "read".to_string(),
        "write".to_string(),
    ])));
    let mut agent = agent(&provider, vec![Arc::new(read), Arc::new(write.clone())])
        .with_offered_tools(offered.clone())
        .with_hooks(Arc::new(WithdrawingTool::new(offered, "write")));

    let (messages, events) = run_agent(&mut agent, Message::user("go")).await;

    assert_eq!(provider.call(0).tool_names(), vec!["read", "write"]);
    assert_eq!(write.call_count(), 0, "a hidden tool must not be runnable");
    assert!(events.tool_ends()[0].3);
    assert!(tool_result_text(&messages[2]).contains("tool not found: write"));
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
async fn an_immediate_provider_error_starts_and_ends_the_assistant_message() {
    let provider = FakeProvider::once(Turn::error("Fake returned 400: bad request"));
    let mut agent = agent(&provider, Vec::new());

    let (_, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert_eq!(events.count("MessageStart"), events.count("MessageEnd"));
    let assistant_start = events
        .iter()
        .position(|event| {
            matches!(
                event,
                micro_types::AgentEvent::MessageStart {
                    message: Message::Assistant(_)
                }
            )
        })
        .expect("the assistant response must start");
    let assistant_end = events
        .iter()
        .position(|event| {
            matches!(
                event,
                micro_types::AgentEvent::MessageEnd {
                    message: Message::Assistant(_)
                }
            )
        })
        .expect("the assistant response must end");
    assert!(
        assistant_start < assistant_end,
        "the response lifecycle is ordered"
    );
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
    let provider = FakeProvider::builder()
        .turns((0..4).map(|turn| {
            Turn::new().with_tool_call(format!("call_{turn}"), "read", json!({ "n": turn }))
        }))
        .build();
    let read = FakeTool::new("read").returning("more");
    let mut agent = agent(&provider, vec![Arc::new(read.clone())]);

    let (_, events) = run_agent(&mut agent, Message::user("keep going")).await;

    assert_eq!(read.call_count(), 4);

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

    assert_eq!(agent.messages(), messages.as_slice());
}

#[tokio::test]
async fn a_turn_that_only_emits_done_still_reports_message_end() {
    let provider = FakeProvider::once(Turn::text("instant").without_deltas());
    let mut agent = agent(&provider, Vec::new());

    let (messages, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert!(events.deltas().is_empty());
    assert_eq!(events.assistant_message_ends().len(), 1);
    assert_eq!(events.assistant_message_ends()[0].text(), "instant");
    assert_eq!(messages.len(), 2);

    assert_eq!(events.names().last(), Some(&"AgentSettled"));
}

#[tokio::test(start_paused = true)]

async fn a_retry_does_not_reopen_the_assistant_message() {
    let provider = FakeProvider::builder()
        .turn(Turn::error("Fake returned 429: slow down").with_start(true))
        .turn(Turn::text("recovered"))
        .build();
    let mut agent = agent(&provider, Vec::new());

    let (_, events) = run_agent(&mut agent, Message::user("hi")).await;

    assert_eq!(events.retries().len(), 1);

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

/// A tool that takes a while, so a batch of them shows whether they ran together.
struct SlowTool {
    name: String,
    holds_for: std::time::Duration,
    /// What this tool asks for via pi's `executionMode`.
    execution_mode: Option<micro_types::ToolExecutionMode>,
}

impl SlowTool {
    fn new(name: impl Into<String>, holds_for: std::time::Duration) -> Self {
        SlowTool {
            name: name.into(),
            holds_for,
            execution_mode: None,
        }
    }

    fn sequential(self) -> Self {
        self.with_execution_mode(micro_types::ToolExecutionMode::Sequential)
    }

    fn with_execution_mode(mut self, mode: micro_types::ToolExecutionMode) -> Self {
        self.execution_mode = Some(mode);
        self
    }
}

#[async_trait::async_trait]
impl micro_tools::Tool for SlowTool {
    fn definition(&self) -> micro_types::ToolDefinition {
        micro_types::ToolDefinition {
            name: self.name.clone(),
            description: "waits".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            constrained_sampling: None,
        }
    }

    fn execution_mode(&self) -> Option<micro_types::ToolExecutionMode> {
        self.execution_mode
    }

    async fn execute(&self, _arguments: &serde_json::Value) -> Result<String, String> {
        tokio::time::sleep(self.holds_for).await;
        Ok(format!("{} is done", self.name))
    }
}

/// The calls in one answer do not depend on each other, so they run together.
#[tokio::test]
async fn the_tools_in_one_answer_run_together() {
    let hold = std::time::Duration::from_millis(200);
    let tools: Vec<Arc<dyn micro_tools::Tool>> = ["one", "two", "three"]
        .into_iter()
        .map(|name| Arc::new(SlowTool::new(name, hold)) as Arc<dyn micro_tools::Tool>)
        .collect();

    let provider = FakeProvider::builder()
        .turn(
            Turn::new()
                .with_tool_call("c1", "one", json!({}))
                .with_tool_call("c2", "two", json!({}))
                .with_tool_call("c3", "three", json!({})),
        )
        .turn(Turn::text("all done"))
        .build();
    let mut agent = agent(&provider, tools);

    let started = std::time::Instant::now();
    let (messages, _) = run_agent(&mut agent, Message::user("go")).await;
    let took = started.elapsed();

    assert!(
        took < hold * 2,
        "parallel tools took {took:?} with a {hold:?} delay",
    );

    let answered: Vec<String> = messages
        .iter()
        .filter_map(|message| match message {
            Message::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(answered, vec!["c1", "c2", "c3"]);
}

/// A tool with no opinion on how it is scheduled is explicitly `Parallel`, not merely the absence
/// of `Sequential`.
#[tokio::test]
async fn tools_explicitly_marked_parallel_still_run_together() {
    let hold = std::time::Duration::from_millis(200);
    let tools: Vec<Arc<dyn micro_tools::Tool>> = ["one", "two", "three"]
        .into_iter()
        .map(|name| {
            Arc::new(
                SlowTool::new(name, hold)
                    .with_execution_mode(micro_types::ToolExecutionMode::Parallel),
            ) as Arc<dyn micro_tools::Tool>
        })
        .collect();

    let provider = FakeProvider::builder()
        .turn(
            Turn::new()
                .with_tool_call("c1", "one", json!({}))
                .with_tool_call("c2", "two", json!({}))
                .with_tool_call("c3", "three", json!({})),
        )
        .turn(Turn::text("all done"))
        .build();
    let mut agent = agent(&provider, tools);

    let started = std::time::Instant::now();
    run_agent(&mut agent, Message::user("go")).await;
    let took = started.elapsed();

    assert!(
        took < hold * 2,
        "explicitly parallel tools took {took:?} with a {hold:?} delay",
    );
}

#[tokio::test]
async fn a_sequential_tool_does_not_overlap_another() {
    let hold = std::time::Duration::from_millis(200);
    let one = SlowTool::new("one", hold);
    let two = SlowTool::new("two", hold).sequential();
    let three = SlowTool::new("three", hold);
    let tools: Vec<Arc<dyn micro_tools::Tool>> =
        vec![Arc::new(one), Arc::new(two), Arc::new(three)];

    let provider = FakeProvider::builder()
        .turn(
            Turn::new()
                .with_tool_call("c1", "one", json!({}))
                .with_tool_call("c2", "two", json!({}))
                .with_tool_call("c3", "three", json!({})),
        )
        .turn(Turn::text("all done"))
        .build();
    let mut agent = agent(&provider, tools);

    let started = std::time::Instant::now();
    let (messages, events) = run_agent(&mut agent, Message::user("go")).await;
    let took = started.elapsed();

    assert!(
        took >= hold * 3 - std::time::Duration::from_millis(30),
        "sequential tools completed too quickly: {took:?} with a {hold:?} delay",
    );

    let ends: Vec<&str> = events.tool_ends().into_iter().map(|(id, ..)| id).collect();
    assert_eq!(ends, vec!["c1", "c2", "c3"]);

    let answered: Vec<String> = messages
        .iter()
        .filter_map(|message| match message {
            Message::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(answered, vec!["c1", "c2", "c3"]);
}

/// One sequential tool in a batch forces every call in that batch to run one at a time.
#[tokio::test]
async fn a_mixed_batch_runs_every_call_one_at_a_time_not_only_the_sequential_one() {
    let hold = std::time::Duration::from_millis(150);

    let tools: Vec<Arc<dyn micro_tools::Tool>> = vec![
        Arc::new(SlowTool::new("one", hold)),
        Arc::new(SlowTool::new("two", hold).sequential()),
        Arc::new(SlowTool::new("three", hold)),
    ];

    let provider = FakeProvider::builder()
        .turn(
            Turn::new()
                .with_tool_call("c1", "one", json!({}))
                .with_tool_call("c2", "two", json!({}))
                .with_tool_call("c3", "three", json!({})),
        )
        .turn(Turn::text("all done"))
        .build();
    let mut agent = agent(&provider, tools);

    let started = std::time::Instant::now();
    run_agent(&mut agent, Message::user("go")).await;
    let took = started.elapsed();

    assert!(
        took >= hold * 3 - std::time::Duration::from_millis(30),
        "one sequential tool in the batch took {took:?}, which is not every call waiting \
         its turn",
    );
}

/// A turn that failed still ends.
#[tokio::test]
async fn a_failed_turn_still_reports_its_end() {
    let provider = FakeProvider::once(Turn::error("the provider refused"));
    let mut agent = agent(&provider, Vec::new());

    let (_, events) = run_agent(&mut agent, Message::user("go")).await;

    let starts = events
        .iter()
        .filter(|event| matches!(event, micro_types::AgentEvent::TurnStart))
        .count();
    let ends = events
        .iter()
        .filter(|event| matches!(event, micro_types::AgentEvent::TurnEnd { .. }))
        .count();
    assert_eq!(
        starts, ends,
        "every turn that started also ended: {events:?}"
    );
    assert!(starts > 0, "a turn did start");
}

/// A message left for a running turn reaches the model at the next turn, without a second run being
/// started for it.
#[tokio::test]
async fn steering_reaches_the_run_that_is_already_going() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("c1", "slow", json!({})))
        .turn(Turn::text("done"))
        .build();
    let tool: Arc<dyn micro_tools::Tool> =
        Arc::new(SlowTool::new("slow", std::time::Duration::from_millis(150)));
    let mut agent = agent(&provider, vec![tool]);

    let steering = agent.steering();
    let steering_task = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        steering.steer(Message::user("actually, stop and summarize"));
    });

    let (messages, _) = run_agent(&mut agent, Message::user("go")).await;
    steering_task.await.unwrap();

    let said: Vec<String> = messages
        .iter()
        .filter_map(|message| match message {
            Message::User { .. } => Some(
                message
                    .content()
                    .iter()
                    .map(micro_types::ContentBlock::as_text)
                    .collect(),
            ),
            _ => None,
        })
        .collect();
    assert!(
        said.iter().any(|text| text.contains("summarize")),
        "what was said mid-run joined the conversation: {said:?}",
    );
}

#[tokio::test]
async fn a_follow_up_continues_the_same_run() {
    let provider = FakeProvider::builder()
        .turn(Turn::text("first"))
        .turn(Turn::text("second"))
        .build();
    let mut agent = agent(&provider, Vec::new());

    let steering = agent.steering();
    steering.follow_up(Message::user("and another thing"));

    let (_, events) = run_agent(&mut agent, Message::user("go")).await;

    let ends = events
        .iter()
        .filter(|event| matches!(event, micro_types::AgentEvent::AgentEnd { .. }))
        .count();
    let turns = events
        .iter()
        .filter(|event| matches!(event, micro_types::AgentEvent::TurnStart))
        .count();
    assert_eq!(ends, 1, "one run, not two");
    assert_eq!(turns, 2, "and it took two turns to get through both");
    assert!(steering.is_empty(), "the queue was drained");
}

struct Deciding(micro_agent::ToolDecision);

struct WithdrawingTool {
    offered: Arc<RwLock<Option<Vec<String>>>>,
    name: String,
}

impl WithdrawingTool {
    fn new(offered: Arc<RwLock<Option<Vec<String>>>>, name: impl Into<String>) -> Self {
        WithdrawingTool {
            offered,
            name: name.into(),
        }
    }
}

#[async_trait::async_trait]
impl micro_agent::Hooks for WithdrawingTool {
    async fn before_tool(
        &self,
        _id: &str,
        _name: &str,
        _arguments: &serde_json::Value,
    ) -> micro_agent::ToolDecision {
        let mut offered = self.offered.write().expect("offered tools lock");
        if let Some(names) = offered.as_mut() {
            names.retain(|name| name != &self.name);
        }
        micro_agent::ToolDecision::Proceed
    }
}

#[async_trait::async_trait]
impl micro_agent::Hooks for Deciding {
    async fn before_tool(
        &self,
        _id: &str,
        _name: &str,
        _arguments: &serde_json::Value,
    ) -> micro_agent::ToolDecision {
        self.0.clone()
    }
}

/// Rewritten arguments are the ones the tool is handed, not the ones the model wrote.
#[tokio::test]
async fn a_hook_can_rewrite_a_call_before_it_runs() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("c1", "read", json!({ "path": "asked.txt" })))
        .turn(Turn::text("done"))
        .build();
    let read = FakeTool::new("read").returning("contents");

    let mut agent = agent(&provider, vec![Arc::new(read.clone())]).with_hooks(Arc::new(Deciding(
        micro_agent::ToolDecision::Rewrite(json!({ "path": "instead.txt" })),
    )));

    let (_, events) = run_agent(&mut agent, Message::user("read it")).await;

    assert_eq!(read.call_count(), 1, "the call still ran");
    assert_eq!(
        read.call(0),
        json!({ "path": "instead.txt" }),
        "the tool was handed the rewritten arguments"
    );

    let announced = events
        .events()
        .iter()
        .find_map(|event| match event {
            micro_types::AgentEvent::ToolStart { arguments, .. } => Some(arguments.clone()),
            _ => None,
        })
        .expect("the call was announced");
    assert_eq!(announced, json!({ "path": "instead.txt" }));
}

/// A refusal stops the call and takes the place of the output the tool would have given.
#[tokio::test]
async fn a_hook_can_refuse_a_call() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("c1", "read", json!({ "path": "secret.txt" })))
        .turn(Turn::text("understood"))
        .build();
    let read = FakeTool::new("read").returning("contents");

    let mut agent = agent(&provider, vec![Arc::new(read.clone())]).with_hooks(Arc::new(Deciding(
        micro_agent::ToolDecision::Refuse("not that one".to_string()),
    )));

    let (messages, _) = run_agent(&mut agent, Message::user("read it")).await;

    assert_eq!(read.call_count(), 0, "the tool never ran");
    assert!(
        messages
            .iter()
            .any(|message| tool_result_text(message).contains("not that one")),
        "the model was told why instead of getting output"
    );
}

/// Doing nothing is the default, and leaves the call exactly as the model wrote it.
#[tokio::test]
async fn a_hook_that_proceeds_changes_nothing() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("c1", "read", json!({ "path": "asked.txt" })))
        .turn(Turn::text("done"))
        .build();
    let read = FakeTool::new("read").returning("contents");

    let mut agent = agent(&provider, vec![Arc::new(read.clone())])
        .with_hooks(Arc::new(Deciding(micro_agent::ToolDecision::Proceed)));

    run_agent(&mut agent, Message::user("read it")).await;

    assert_eq!(read.call_count(), 1);
    assert_eq!(read.call(0), json!({ "path": "asked.txt" }));
}

#[tokio::test]
async fn every_request_carries_the_credential_the_store_holds_now() {
    let root = std::env::temp_dir().join("micro-testkit-credential-per-request");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let store = Arc::new(micro_auth::AuthStore::open_at(root.join("auth.json")).unwrap());
    store
        .set("anthropic", micro_auth::Credential::api_key("first"))
        .unwrap();

    let provider = FakeProvider::builder()
        .turn(Turn::text("one"))
        .turn(Turn::text("two"))
        .build();
    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        Vec::new(),
        model(),
        micro_provider::ApiKey::Stored {
            store: Arc::clone(&store),
            provider: "anthropic".into(),
            resolved: "first".into(),
        },
    );

    run_agent(&mut agent, Message::user("before")).await;
    store
        .set("anthropic", micro_auth::Credential::api_key("second"))
        .unwrap();
    run_agent(&mut agent, Message::user("after")).await;

    assert_eq!(provider.call(0).api_key, "first");
    assert_eq!(provider.call(1).api_key, "second");
}

/// A credential the store cannot produce, with nothing in hand to fall back on, stops the turn
/// where it stands. Sending an unauthenticated request only earns a complaint about the header,
/// which tells nobody that the credential is what went wrong.
#[tokio::test]
async fn a_turn_with_no_credential_at_all_is_not_sent() {
    let root = std::env::temp_dir().join("micro-testkit-credential-missing");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let store = Arc::new(micro_auth::AuthStore::open_at(root.join("auth.json")).unwrap());
    let provider = FakeProvider::builder()
        .turn(Turn::text("never asked"))
        .build();
    let mut agent = Agent::new(
        Arc::new(provider.clone()),
        Vec::new(),
        model(),
        micro_provider::ApiKey::Stored {
            store,
            provider: "github-copilot".into(),
            resolved: String::new(),
        },
    );

    let (messages, _) = run_agent(&mut agent, Message::user("go")).await;

    assert!(provider.calls().is_empty(), "nothing was sent");
    let said = messages
        .iter()
        .filter_map(|message| match message {
            Message::Assistant(assistant) => assistant.error.clone(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        said.contains("no credential for github-copilot"),
        "the turn says what is missing: {said}"
    );
}

/// A deferred tool is not described to the model and is still callable.
#[tokio::test]
async fn a_deferred_tool_is_hidden_from_the_model_but_still_runs() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("c1", "hidden", json!({})))
        .turn(Turn::text("done"))
        .build();
    let hidden = FakeTool::new("hidden").returning("it ran anyway");
    let plain = FakeTool::new("plain").returning("ordinary");

    let mut agent = agent(
        &provider,
        vec![
            Arc::new(micro_tools::Deferred::new(Arc::new(hidden.clone()))),
            Arc::new(plain.clone()),
        ],
    );

    let (messages, _) = run_agent(&mut agent, Message::user("use it")).await;

    let calls = provider.calls();
    let advertised = calls[0].tool_names();
    assert!(
        !advertised.contains(&"hidden"),
        "a deferred tool is not described: {advertised:?}"
    );
    assert!(advertised.contains(&"plain"), "{advertised:?}");

    assert_eq!(hidden.call_count(), 1, "and it still ran when asked for");
    assert!(
        messages
            .iter()
            .any(|message| tool_result_text(message).contains("it ran anyway")),
        "its output reached the model"
    );
}

/// A provider whose stream a test controls by hand.
struct PausedProvider {
    stream: Mutex<Option<UnboundedReceiver<StreamEvent>>>,
}

impl Provider for PausedProvider {
    fn name(&self) -> &str {
        "paused"
    }

    fn stream(
        &self,
        _model: Model,
        _context: Context,
        _api_key: String,
    ) -> UnboundedReceiver<StreamEvent> {
        self.stream
            .lock()
            .expect("stream lock")
            .take()
            .expect("stream() is called once per turn")
    }

    fn payload(&self, _model: &Model, _context: &Context) -> serde_json::Value {
        serde_json::Value::Null
    }
}

/// A turn abandoned mid-answer still reports that the run ended.
#[tokio::test]
async fn an_interrupted_turn_still_settles() {
    let (stream_tx, stream_rx) = tokio::sync::mpsc::unbounded_channel();
    let provider = Arc::new(PausedProvider {
        stream: Mutex::new(Some(stream_rx)),
    });
    let mut agent = Agent::new(provider, Vec::new(), model(), "test-key");

    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut turn = Box::pin(agent.run(Message::user("go"), &events_tx));

    stream_tx
        .send(StreamEvent::Start)
        .expect("the agent is still listening");

    tokio::select! {
        _ = &mut turn => panic!("the turn should not be able to finish without more from the stream"),
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
    }

    drop(turn);

    let mut settled = false;
    let mut ended = false;
    while let Ok(event) = events_rx.try_recv() {
        match event {
            micro_types::AgentEvent::AgentEnd { .. } => ended = true,
            micro_types::AgentEvent::AgentSettled => settled = true,
            _ => {}
        }
    }
    assert!(ended, "an interrupted turn should still report AgentEnd");
    assert!(
        settled,
        "an interrupted turn should still report AgentSettled"
    );
}
