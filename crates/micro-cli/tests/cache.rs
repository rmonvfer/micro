//! What the binary actually sends, turn after turn.

mod support;

use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use support::FakeApi;
use support::Fixture;
use support::Reply;

/// Every ledger event a session recorded, in order, as `(seq, type, event)`.
fn ledger(fixture: &Fixture) -> Vec<(u64, String, Value)> {
    fixture
        .session_logs()
        .iter()
        .flat_map(|log| log.lines().map(str::to_string).collect::<Vec<_>>())
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter_map(|line| {
            let event = line.get("event")?.clone();
            let kind = event.get("type")?.as_str()?.to_string();
            Some((line.get("seq")?.as_u64()?, kind, event))
        })
        .collect()
}

fn events_named<'a>(
    recorded: &'a [(u64, String, Value)],
    kind: &str,
) -> Vec<&'a (u64, String, Value)> {
    recorded
        .iter()
        .filter(|(_, named, _)| named == kind)
        .collect()
}

/// The one session this fixture recorded, by id.
fn session_id(fixture: &Fixture) -> String {
    let directory = fixture.home().join("sessions");
    let mut ids: Vec<String> = std::fs::read_dir(&directory)
        .expect("the session directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "jsonl"))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .collect();
    ids.sort();
    ids.pop().expect("a session was recorded")
}

/// The system message an OpenAI-shaped request opens with.
fn system_message(request: &Value) -> Value {
    request["messages"][0].clone()
}

fn messages(request: &Value) -> Vec<Value> {
    request["messages"].as_array().cloned().unwrap_or_default()
}

/// Two turns of one run open with the same bytes, and the session says the same.
#[test]
fn a_second_turn_sends_the_first_turns_prefix_unchanged() {
    let api = FakeApi::start([
        Reply::tool_call("call_1", "ls", serde_json::json!({ "path": "." })),
        Reply::text("done"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write("AGENTS.md", "Run the linter before you finish.");

    fixture
        .print(&["-m", "test", "list the files"])
        .expect_success("micro --print");

    assert_eq!(api.request_count(), 2, "a tool call means a second turn");
    let first = api.request(0);
    let second = api.request(1);

    assert_eq!(
        system_message(&first),
        system_message(&second),
        "the system prompt moved between two turns of one run"
    );
    assert_eq!(first["tools"], second["tools"], "the tools moved");

    
    let (had, has) = (messages(&first), messages(&second));
    assert!(has.len() > had.len(), "the conversation grew");
    assert_eq!(has[..had.len()], had[..], "and only at the end");

    let recorded = ledger(&fixture);
    let hashes: Vec<String> = events_named(&recorded, "turn_request")
        .iter()
        .map(|(_, _, event)| {
            event["prefix_hash"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert_eq!(hashes.len(), 2);
    assert_eq!(hashes[0], hashes[1], "the ledger agrees the prefix held");
    assert!(
        events_named(&recorded, "prefix_changed").is_empty(),
        "and nothing claimed it changed"
    );

    
    let explained = fixture.micro_run(&["why-miss", &session_id(&fixture), "2"]);
    explained.expect_success("micro why-miss");
    assert!(
        explained.stdout.contains("The prefix held"),
        "why-miss said: {}",
        explained.stdout
    );
}


#[test]
fn reloading_mid_session_reaches_the_next_request_and_why_miss_names_the_span() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipped: python3 is not on the path, and the terminal driver needs it");
        return;
    }

    let api = FakeApi::start([Reply::text("first answer"), Reply::text("second answer")]);
    let fixture = Fixture::new(&api);
    fixture.write("AGENTS.md", "Always run the linter.");
    
    if micro_extensions::which_bun().is_some() {
        fixture.write(
            ".micro/extensions/noop.ts",
            "export default function (pi) {\n\
             \tpi.registerCommand(\"noop\", {\n\
             \t\tdescription: \"does nothing at all\",\n\
             \t\thandler: async () => {},\n\
             \t});\n\
             }\n",
        );
    }

    let mut command = pty_command(&fixture, &["-m", "test"]);
    command.env(
        "KEYS",
        [
            "hello\r",
            "!printf 'Always run the tests.' > AGENTS.md\r",
            "/reload\r",
            "hello again\r",
        ]
        .join("~~"),
    );
    command.env("GAP", "2.5");
    command.env("WAIT", "17");
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let driven = command.output().expect("the terminal driver runs");
    assert!(
        !driven.stdout.is_empty(),
        "the driver produced nothing: {}",
        String::from_utf8_lossy(&driven.stderr)
    );

    assert_eq!(
        api.request_count(),
        2,
        "both prompts should have reached the provider"
    );
    let before = api.request(0);
    let after = api.request(1);

    let said = |request: &Value| {
        system_message(request)["content"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    assert!(
        said(&before).contains("Always run the linter."),
        "the first request carried the instructions as they were: {}",
        said(&before)
    );
    assert!(
        said(&after).contains("Always run the tests."),
        "and the second carried them as they are now: {}",
        said(&after)
    );

    let recorded = ledger(&fixture);
    let changes = events_named(&recorded, "prefix_changed");
    assert_eq!(changes.len(), 1, "recorded once: {changes:?}");
    assert_eq!(changes[0].2["reason"], "reload");
    assert_ne!(changes[0].2["from_hash"], changes[0].2["to_hash"]);

    let explained = fixture.micro_run(&["why-miss", &session_id(&fixture), "2"]);
    explained.expect_success("micro why-miss");
    let printed = &explained.stdout;
    assert!(
        printed.contains("project_instructions"),
        "why-miss should name the span that moved: {printed}"
    );
    assert!(
        printed.contains("- Always run the linter.") && printed.contains("+ Always run the tests."),
        "and show what it used to say: {printed}"
    );
    assert!(
        printed.contains("The cache broke because"),
        "and say why: {printed}"
    );
}

fn drive_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pty/drive.py")
}


fn pty_command(fixture: &Fixture, micro_args: &[&str]) -> Command {
    let base = fixture.micro();
    let mut command = Command::new("python3");
    command.arg(drive_script());
    command.arg("--");
    command.arg(base.get_program());
    if let Some(directory) = base.get_current_dir() {
        command.current_dir(directory);
    }
    for (key, value) in base.get_envs() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    command.args(micro_args);
    command
}
