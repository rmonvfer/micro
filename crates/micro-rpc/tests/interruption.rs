//! What a caller can do to a turn while the turn is still running.

use micro_agent::Agent;
use micro_rpc::Rpc;
use micro_testkit::FakeProvider;
use micro_testkit::Turn;
use micro_types::Model;
use micro_types::ThinkingLevel;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::Mutex;

struct SlowTool;

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
    let root = std::env::temp_dir().join(format!("micro-rpc-test-{}", std::process::id()));
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
