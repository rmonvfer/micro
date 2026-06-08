//! What a session recorded about itself, read back through the binary.

mod support;

use serde_json::json;
use serde_json::Value;
use support::FakeApi;
use support::Fixture;
use support::Output;
use support::Reply;

/// The id of the session the last run wrote, as the binary itself lists it.
fn only_session(fixture: &Fixture) -> String {
    let listed = Output::run(fixture.micro().args(["sessions", "list"]));
    listed.expect_success("micro sessions list");
    listed
        .stdout
        .split_whitespace()
        .next()
        .expect("a session id")
        .to_string()
}

#[test]
fn a_recorded_turn_rebuilds_the_request_that_was_sent() {
    let api = FakeApi::start([
        Reply::tool_call("call_1", "read", json!({ "path": "notes.txt" })),
        Reply::text("read it"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write("notes.txt", "the contents");
    fixture.write("AGENTS.md", "Prefer short answers.");

    fixture
        .print(&["-m", "test", "read notes.txt"])
        .expect_success("micro --print");

    let id = only_session(&fixture);
    let shown = Output::run(
        fixture
            .micro()
            .args(["sessions", "show", &id, "--turn", "2", "--raw"]),
    );
    shown.expect_success("micro sessions show --raw");

    let rebuilt: Value = serde_json::from_str(&shown.stdout).expect("a request body");
    assert_eq!(
        rebuilt,
        api.request(1),
        "the second turn as recorded is not the second turn as sent"
    );
    assert!(
        !shown.stderr.contains("rebuilds to a different request"),
        "the rebuilt request did not hash to the one recorded: {}",
        shown.stderr
    );
}

#[test]
fn a_turn_names_who_supplied_each_stretch_of_the_prompt() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);
    fixture.write("AGENTS.md", "Prefer short answers.");

    fixture
        .print(&["-m", "test", "hello"])
        .expect_success("micro --print");

    let id = only_session(&fixture);
    let shown = Output::run(
        fixture
            .micro()
            .args(["sessions", "show", &id, "--turn", "1"]),
    );
    shown.expect_success("micro sessions show");

    assert!(
        shown.stdout.contains("system_prompt"),
        "got {}",
        shown.stdout
    );
    assert!(
        shown.stdout.contains("project_instructions"),
        "the workspace's own instructions are a span of their own: {}",
        shown.stdout
    );
    assert!(
        shown.stdout.contains("turn 1 of session"),
        "got {}",
        shown.stdout
    );
    assert!(
        shown.stdout.contains("read"),
        "the tools the model was offered are named: {}",
        shown.stdout
    );
}

/// A session with no turn named lists the turns it holds, so a reader knows what there is to ask
/// about.
#[test]
fn a_session_lists_the_turns_it_recorded() {
    let api = FakeApi::start([
        Reply::tool_call("call_1", "read", json!({ "path": "notes.txt" })),
        Reply::text("read it"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write("notes.txt", "the contents");

    fixture
        .print(&["-m", "test", "read notes.txt"])
        .expect_success("micro --print");

    let id = only_session(&fixture);
    let shown = Output::run(fixture.micro().args(["sessions", "show", &id]));
    shown.expect_success("micro sessions show");

    assert!(shown.stdout.contains("turn 1"), "got {}", shown.stdout);
    assert!(shown.stdout.contains("turn 2"), "got {}", shown.stdout);
    assert!(
        shown.stdout.contains("openai/test-model"),
        "each turn says what answered it: {}",
        shown.stdout
    );
}

#[test]
fn exporting_a_session_yields_its_whole_ledger() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);

    fixture
        .print(&["-m", "test", "hello"])
        .expect_success("micro --print");

    let id = only_session(&fixture);
    let exported = Output::run(fixture.micro().args(["sessions", "export", &id]));
    exported.expect_success("micro sessions export");

    let lines: Vec<Value> = exported
        .stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every exported line is JSON"))
        .collect();

    let kinds: Vec<&str> = lines
        .iter()
        .filter_map(|line| line["event"]["type"].as_str())
        .collect();
    assert!(
        kinds.contains(&"turn_request") && kinds.contains(&"turn_usage"),
        "expected the request and what it cost, got {kinds:?}"
    );
    assert!(
        lines.iter().any(|line| line["message"]["role"] == "user"),
        "the conversation is exported alongside the facts"
    );
    for line in &lines {
        if line.get("event").is_some() {
            assert_eq!(line["v"], 1, "every ledger line says which schema it is");
        }
    }
}

#[test]
fn a_session_recorded_before_the_ledger_says_it_has_no_turns() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);
    fixture
        .print(&["-m", "test", "hello"])
        .expect_success("micro --print");

    let id = only_session(&fixture);

    let log = fixture.home().join("sessions").join(format!("{id}.jsonl"));
    let kept: Vec<String> = std::fs::read_to_string(&log)
        .expect("the log")
        .lines()
        .filter(|line| !line.contains("\"event\""))
        .map(str::to_string)
        .collect();
    std::fs::write(&log, kept.join("\n") + "\n").expect("rewrite the log");

    let shown = Output::run(fixture.micro().args(["sessions", "show", &id]));
    shown.expect_success("micro sessions show");
    assert!(
        shown.stdout.contains("No recorded turns"),
        "got {}",
        shown.stdout
    );
}
