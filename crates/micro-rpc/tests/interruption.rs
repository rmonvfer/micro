//! What a caller can do to a turn while the turn is still running.

use micro_agent::Agent;
use micro_rpc::Rpc;
use micro_testkit::FakeProvider;
use micro_testkit::Turn;
use micro_types::Model;
use micro_types::ThinkingLevel;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::Mutex;

struct SlowTool;

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

#[async_trait::async_trait]
impl micro_tools::Tool for SlowTool {
    fn definition(&self) -> micro_types::ToolDefinition {
        micro_types::ToolDefinition {
            name: "slow".into(),
            description: "waits".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            constrained_sampling: None,
        }
    }

    async fn execute(&self, _arguments: &serde_json::Value) -> Result<String, String> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok("finished".into())
    }
}

fn model() -> Model {
    Model {
        id: "test-model".into(),
        provider: "fake".into(),
        base_url: "https://example.invalid".into(),
        max_tokens: 1024,
        thinking: ThinkingLevel::Off,
        reasoning: false,
        compat: Default::default(),
        headers: Default::default(),
    }
}

async fn rpc_with(
    provider: FakeProvider,
    tools: Vec<Arc<dyn micro_tools::Tool>>,
) -> (Rpc, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "micro-rpc-test-{}-{}",
        std::process::id(),
        NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let store = micro_session::SessionStore::new(root.join("sessions"));
    let session = store.create(&root, "test-model").await.unwrap();
    let agent = Agent::new(Arc::new(provider), tools, model(), "test-key");

    (
        Rpc::new(
            agent,
            Arc::new(Mutex::new(session)),
            micro_models::Catalog::bundled(),
            root.clone(),
        ),
        root,
    )
}

/// `abort` reaches a turn that is still running.
#[tokio::test]
async fn abort_stops_a_running_turn() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("c1", "slow", json!({})))
        .turn(Turn::text("done"))
        .build();
    let (mut rpc, _root) = rpc_with(provider, vec![Arc::new(SlowTool)]).await;

    let (mut caller, agent_side) = tokio::io::duplex(64 * 1024);
    let (agent_out, mut reading) = tokio::io::duplex(64 * 1024);

    let running = tokio::spawn(async move { rpc.run(agent_side, agent_out).await });

    caller
        .write_all(b"{\"type\":\"prompt\",\"message\":\"go\",\"id\":\"1\"}\n")
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;
    caller
        .write_all(b"{\"type\":\"abort\",\"id\":\"2\"}\n")
        .await
        .unwrap();

    let started = std::time::Instant::now();
    let mut lines = tokio::io::BufReader::new(&mut reading).lines();
    let mut saw_abort = false;
    while let Ok(Some(line)) = lines.next_line().await {
        if line.contains("\"command\":\"abort\"") {
            saw_abort = true;
            break;
        }
    }

    assert!(saw_abort, "the abort was answered");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "it was answered while the turn was running, not after it",
    );

    drop(caller);
    let _ = tokio::time::timeout(Duration::from_secs(5), running).await;
}

#[tokio::test]
async fn a_follow_up_sent_mid_turn_continues_the_run() {
    let provider = FakeProvider::builder()
        .turn(Turn::text("first"))
        .turn(Turn::text("second"))
        .build();
    let (mut rpc, _root) = rpc_with(provider, Vec::new()).await;

    let (mut caller, agent_side) = tokio::io::duplex(64 * 1024);
    let (agent_out, mut reading) = tokio::io::duplex(64 * 1024);
    let running = tokio::spawn(async move { rpc.run(agent_side, agent_out).await });

    caller
        .write_all(b"{\"type\":\"prompt\",\"message\":\"one\",\"id\":\"1\"}\n")
        .await
        .unwrap();
    caller
        .write_all(b"{\"type\":\"follow_up\",\"message\":\"two\",\"id\":\"2\"}\n")
        .await
        .unwrap();

    let mut lines = tokio::io::BufReader::new(&mut reading).lines();
    let mut turns = 0;
    let mut runs = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(2), lines.next_line()).await {
            Ok(Ok(Some(line))) => {
                if line.contains("\"turn_start\"") {
                    turns += 1;
                }
                if line.contains("\"agent_end\"") {
                    runs += 1;
                    break;
                }
            }
            _ => break,
        }
    }

    assert_eq!(turns, 2, "the prompt and the follow-up each took a turn");
    assert_eq!(runs, 1, "follow-up should stay in one run");
    drop(caller);
    let _ = tokio::time::timeout(Duration::from_secs(5), running).await;
}

/// Read answers until one for `command` arrives, and hand back that one.
async fn answer_to<R>(lines: &mut tokio::io::Lines<R>, command: &str) -> serde_json::Value
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let wanted = format!("\"command\":\"{command}\"");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await {
            Ok(Ok(Some(line))) if line.contains(&wanted) => {
                return serde_json::from_str(&line).unwrap();
            }
            Ok(Ok(Some(_))) => continue,
            _ => break,
        }
    }
    panic!("no answer to {command}");
}

/// Every accepted input says what became of it.
#[tokio::test]
async fn each_input_reports_its_disposition() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("c1", "slow", json!({})))
        .turn(Turn::text("done"))
        .build();
    let (mut rpc, _root) = rpc_with(provider, vec![Arc::new(SlowTool)]).await;

    let (mut caller, agent_side) = tokio::io::duplex(64 * 1024);
    let (agent_out, mut reading) = tokio::io::duplex(64 * 1024);
    let running = tokio::spawn(async move { rpc.run(agent_side, agent_out).await });
    let mut lines = tokio::io::BufReader::new(&mut reading).lines();

    caller
        .write_all(b"{\"type\":\"prompt\",\"message\":\"go\",\"id\":\"1\"}\n")
        .await
        .unwrap();
    let started = answer_to(&mut lines, "prompt").await;
    assert_eq!(started["data"]["disposition"], "started");

    for (kind, id) in [("steer", "2"), ("follow_up", "3"), ("prompt", "4")] {
        let command = format!("{{\"type\":\"{kind}\",\"message\":\"later\",\"id\":\"{id}\"}}\n");
        caller.write_all(command.as_bytes()).await.unwrap();
        let answer = answer_to(&mut lines, kind).await;
        assert_eq!(answer["id"], id);
        assert_eq!(answer["data"]["disposition"], "queued", "{answer}");
    }

    caller
        .write_all(b"{\"type\":\"abort\",\"id\":\"5\"}\n")
        .await
        .unwrap();
    answer_to(&mut lines, "abort").await;
    drop(caller);
    let _ = tokio::time::timeout(Duration::from_secs(5), running).await;
}

/// `clear_queue` hands back what was waiting, and none of it is sent.
#[tokio::test]
async fn clearing_the_queue_returns_what_was_waiting() {
    let provider = FakeProvider::builder()
        .turn(Turn::new().with_tool_call("c1", "slow", json!({})))
        .turn(Turn::text("done"))
        .build();
    let (mut rpc, _root) = rpc_with(provider, vec![Arc::new(SlowTool)]).await;

    let (mut caller, agent_side) = tokio::io::duplex(64 * 1024);
    let (agent_out, mut reading) = tokio::io::duplex(64 * 1024);
    let running = tokio::spawn(async move { rpc.run(agent_side, agent_out).await });
    let mut lines = tokio::io::BufReader::new(&mut reading).lines();

    caller
        .write_all(b"{\"type\":\"prompt\",\"message\":\"go\",\"id\":\"1\"}\n")
        .await
        .unwrap();
    answer_to(&mut lines, "prompt").await;
    caller
        .write_all(b"{\"type\":\"steer\",\"message\":\"change direction\"}\n")
        .await
        .unwrap();
    answer_to(&mut lines, "steer").await;
    caller
        .write_all(b"{\"type\":\"follow_up\",\"message\":\"summarize after\"}\n")
        .await
        .unwrap();
    answer_to(&mut lines, "follow_up").await;
    caller
        .write_all(b"{\"type\":\"prompt\",\"message\":\"and then this\"}\n")
        .await
        .unwrap();
    answer_to(&mut lines, "prompt").await;

    caller
        .write_all(b"{\"type\":\"clear_queue\",\"id\":\"9\"}\n")
        .await
        .unwrap();
    let cleared = answer_to(&mut lines, "clear_queue").await;
    assert_eq!(cleared["id"], "9");
    assert_eq!(cleared["data"]["steering"], json!(["change direction"]));
    assert_eq!(
        cleared["data"]["follow_up"],
        json!(["summarize after", "and then this"])
    );

    caller
        .write_all(b"{\"type\":\"clear_queue\"}\n")
        .await
        .unwrap();
    let again = answer_to(&mut lines, "clear_queue").await;
    assert_eq!(again["data"]["steering"], json!([]));
    assert_eq!(again["data"]["follow_up"], json!([]));

    drop(caller);
    let _ = tokio::time::timeout(Duration::from_secs(5), running).await;
}
