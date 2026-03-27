//! A machine with no session behind it, for checking the wire against a real phone.
//!
//! It pairs, connects, offers a made-up session, and answers whatever arrives the way the
//! bridge would. What it is for is the one thing the crate's own tests cannot do: proving
//! that the phone's implementation and this one agree, rather than that this one agrees
//! with itself.
//!
//! ```text
//! cd locally && DB_PATH=:memory: PORT=8090 bun run relay/src/main.ts
//! cargo run -p micro-remote --example machine -- http://localhost:8090
//! # then type the printed code into the app, or pair phone-sim with a link
//! ```

use micro_remote::AvailableModel;
use micro_remote::Bridge;
use micro_remote::Delivery;
use micro_remote::MachinePayload;
use micro_remote::RelayClient;
use micro_remote::RelayConfig;
use micro_remote::RelayEvent;
use micro_remote::Session;
use micro_remote::SessionState;
use micro_remote::SlashCommand;
use serde_json::json;
use serde_json::Value;

/// A session that does nothing but answer.
struct Stub;

impl Session for Stub {
    fn submit(&mut self, text: &str, delivery: Delivery) -> Result<(), String> {
        println!("  the phone submitted ({delivery:?}): {text}");
        Ok(())
    }

    fn abort(&mut self) {
        println!("  the phone asked to stop the turn");
    }

    fn is_idle(&self) -> bool {
        true
    }

    fn entries(&self) -> Vec<Value> {
        vec![json!({
            "type": "message",
            "id": "entry-0",
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": "hello from the machine" }],
                "timestamp": 1_772_445_600_000i64,
            },
        })]
    }

    fn state(&self) -> SessionState {
        SessionState {
            model: "claude-sonnet-5".into(),
            provider: "anthropic".into(),
            thinking_level: "medium".into(),
            session_name: "a demonstration".into(),
            cwd: "/work".into(),
            is_streaming: false,
        }
    }

    fn available_models(&self) -> Vec<AvailableModel> {
        vec![AvailableModel {
            id: "anthropic/claude-sonnet-5".into(),
            name: "Sonnet 5".into(),
            provider: "anthropic".into(),
        }]
    }

    fn commands(&self) -> Vec<SlashCommand> {
        vec![SlashCommand {
            name: "compact".into(),
            description: "Compact this session's context".into(),
        }]
    }

    fn set_model(&mut self, model_id: &str) -> Result<(), String> {
        println!("  the phone chose the model {model_id}");
        Ok(())
    }

    fn set_thinking_level(&mut self, level: &str) -> Result<(), String> {
        println!("  the phone chose the thinking level {level}");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let relay = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://localhost:8090".to_string());
    let directory = std::env::temp_dir().join("micro-remote-example");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;

    let path = micro_remote::path_in(&directory);
    let pairing = match micro_remote::load_pairing(&path) {
        Some(pairing) => pairing,
        None => {
            // The same exchange `/remote pair` runs: publish a public half under a short
            // code, wait for somebody to type it, and arrive at the shared secret.
            let enrolment = micro_remote::begin_enrolment(&relay).await?;
            println!("\n  Pairing code:  {}\n", enrolment.code);
            println!("  Type it into Parley on your phone.\n");
            let secret = enrolment.complete().await?;
            micro_remote::write_pairing(&path, &relay, enrolment.pairing_id(), &secret)
                .map_err(|error| error.to_string())?
        }
    };
    let secret = pairing.secret().ok_or("the pairing is unreadable")?;
    let config = RelayConfig {
        relay_url: pairing.relay_url.clone(),
        pairing_id: pairing.pairing_id.clone(),
        secret,
        session_id: "s1".into(),
    };

    micro_remote::register(&config).await?;
    println!("paired with {}", pairing.machine_name);

    let (events, mut incoming) = tokio::sync::mpsc::unbounded_channel();
    let client = RelayClient::start(config, events);
    let bridge = Bridge::new("s1");
    let mut session = Stub;

    while let Some(event) = incoming.recv().await {
        match event {
            RelayEvent::State(state) => println!("relay: {state:?}"),
            RelayEvent::Peer { connected } => {
                println!("phone: {}", if connected { "here" } else { "gone" });
                if connected {
                    client.send(MachinePayload::SessionOffer {
                        session_id: "s1".into(),
                        session_name: "a demonstration".into(),
                        cwd: "/work".into(),
                        machine_name: pairing.machine_name.clone(),
                    });
                    if std::env::args().any(|argument| argument == "--replay") {
                        replay(&client, &bridge).await;
                    }
                }
            }
            RelayEvent::Payload(payload) => {
                println!("phone asked: {payload:?}");
                client.send(bridge.handle(&mut session, payload));
            }
        }
    }
    Ok(())
}

/// A turn, played out at the speed one happens, for looking at what the phone draws.
///
/// The events are the shapes micro's own translator emits, so what the phone is shown
/// here is what it is shown by a real session — which is the only reason a scripted one
/// is worth anything.
async fn replay(client: &RelayClient, bridge: &Bridge) {
    async fn beat(millis: u64) {
        tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
    }

    let send = |event: Value| client.send(bridge.mirror(event));

    send(json!({ "type": "agent_start" }));
    send(json!({ "type": "turn_start", "turnIndex": 0, "timestamp": 1_772_445_600_000i64 }));
    send(json!({
        "type": "message_start",
        "message": {
            "role": "user",
            "content": [{ "type": "text", "text": "why is the timeline scrolling badly on long sessions?" }],
            "timestamp": 1_772_445_600_000i64,
        },
    }));
    beat(400).await;

    // Reasoning, streamed the way a model produces it.
    send(json!({ "type": "message_update", "assistantMessageEvent": { "type": "start" } }));
    send(json!({
        "type": "message_update",
        "assistantMessageEvent": { "type": "thinking_start", "contentIndex": 0 },
    }));
    for piece in ["Long sessions mean many rows. ", "The list is probably rebuilding every frame."] {
        send(json!({
            "type": "message_update",
            "assistantMessageEvent": { "type": "thinking_delta", "contentIndex": 0, "delta": piece },
        }));
        beat(500).await;
    }
    send(json!({
        "type": "message_update",
        "assistantMessageEvent": { "type": "thinking_end", "contentIndex": 0 },
    }));

    // A command, from call to output.
    send(json!({
        "type": "tool_execution_start",
        "toolCallId": "call_1",
        "toolName": "bash",
        "args": { "command": "rg -n 'LazyVStack' app/Parley --stats" },
    }));
    beat(1_400).await;
    send(json!({
        "type": "tool_execution_end",
        "toolCallId": "call_1",
        "toolName": "bash",
        "result": { "content": [{ "type": "text", "text": "TranscriptView.swift:60: LazyVStack(alignment: .leading, spacing: 0) {\n\n1 matches\n1 matched lines\n1 files contained matches" }] },
        "isError": false,
    }));

    // Reading a file, which reads as what it explored rather than as a tool call.
    send(json!({
        "type": "tool_execution_start",
        "toolCallId": "call_2",
        "toolName": "read",
        "args": { "path": "app/Parley/Features/Session/TranscriptView.swift" },
    }));
    beat(900).await;
    send(json!({
        "type": "tool_execution_end",
        "toolCallId": "call_2",
        "toolName": "read",
        "result": { "content": [{ "type": "text", "text": "…82 lines…" }] },
        "isError": false,
    }));

    // An edit, which carries its own patch and tally.
    send(json!({
        "type": "tool_execution_start",
        "toolCallId": "call_3",
        "toolName": "edit",
        "args": {
            "path": "app/Parley/Features/Session/TranscriptView.swift",
            "old_string": "LazyVStack(alignment: .leading, spacing: 0) {",
            "new_string": "LazyVStack(alignment: .leading, spacing: 0) {\n    // Rows are keyed so the list reuses them across updates.",
        },
    }));
    beat(1_200).await;
    send(json!({
        "type": "tool_execution_end",
        "toolCallId": "call_3",
        "toolName": "edit",
        "result": { "content": [{ "type": "text", "text": "Edited app/Parley/Features/Session/TranscriptView.swift" }] },
        "isError": false,
    }));

    // A command that fails, so a failed row can be seen next to the finished ones.
    send(json!({
        "type": "tool_execution_start",
        "toolCallId": "call_4",
        "toolName": "bash",
        "args": { "command": "swift test --filter TranscriptViewTests" },
    }));
    beat(1_100).await;
    send(json!({
        "type": "tool_execution_end",
        "toolCallId": "call_4",
        "toolName": "bash",
        "result": { "content": [{ "type": "text", "text": "error: no such filter 'TranscriptViewTests'" }] },
        "isError": true,
    }));

    // The answer.
    send(json!({
        "type": "message_update",
        "assistantMessageEvent": { "type": "text_start", "contentIndex": 1 },
    }));
    for piece in [
        "The list rebuilds every row on each update because nothing keys them.\n\n",
        "I added a comment marking the fix site in `TranscriptView.swift`. ",
        "The real change is to give each row a stable identity so `LazyVStack` can reuse it.",
    ] {
        send(json!({
            "type": "message_update",
            "assistantMessageEvent": { "type": "text_delta", "contentIndex": 1, "delta": piece },
        }));
        beat(450).await;
    }
    send(json!({
        "type": "message_update",
        "assistantMessageEvent": { "type": "text_end", "contentIndex": 1 },
    }));
    send(json!({ "type": "message_end", "message": { "role": "assistant" } }));
    send(json!({ "type": "turn_end", "turnIndex": 0 }));
    send(json!({ "type": "agent_settled" }));
}
