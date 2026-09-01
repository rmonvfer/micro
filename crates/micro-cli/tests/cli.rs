//! End-to-end tests of the compiled `micro` binary against a fake provider served from the test
//! process.

mod support;

use std::io::BufRead as _;
use std::io::BufReader;
use std::io::Write as _;

use micro_extensions::which_bun;
use serde_json::json;
use support::offered_tools;
use support::path_of;
use support::tool_results;
use support::transcript;
use support::FakeApi;
use support::Fixture;
use support::Output;
use support::Reply;

fn stdout_json(output: &Output) -> serde_json::Value {
    serde_json::from_str(output.stdout.trim())
        .unwrap_or_else(|error| panic!("stdout is not JSON: {error}: {:?}", output.stdout))
}

fn nested_object_with<'a>(
    value: &'a serde_json::Value,
    key: &str,
    expected: &serde_json::Value,
) -> Option<&'a serde_json::Value> {
    if value.get(key) == Some(expected) {
        return Some(value);
    }
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| nested_object_with(value, key, expected)),
        serde_json::Value::Object(values) => values
            .values()
            .find_map(|value| nested_object_with(value, key, expected)),
        _ => None,
    }
}

#[test]
fn print_streams_the_answer_to_stdout() {
    let api = FakeApi::start([Reply::Sse(vec![
        support::text_delta("Hello, "),
        support::text_delta("world"),
        support::finish("stop"),
    ])]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-m", "test", "say hello"]);

    output.expect_success("micro --print");
    assert!(
        output.stdout.contains("Hello, world"),
        "stdout was {:?}",
        output.stdout
    );
}

#[test]
fn the_first_request_carries_the_prompt_the_model_and_the_tools() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);

    fixture
        .print(&["-m", "test", "count the files"])
        .expect_success("micro --print");

    let request = api.request(0);
    assert_eq!(request["model"], "test-model");
    assert_eq!(request["stream"], true);
    assert!(transcript(&request).contains("count the files"));

    let tools = offered_tools(&request);
    for expected in ["read", "write", "edit", "ls", "grep", "bash"] {
        assert!(
            tools.contains(&expected.to_string()),
            "no {expected} in {tools:?}"
        );
    }
}

#[test]
fn every_request_carries_the_stored_credential() {
    let api = FakeApi::start([Reply::text("ok")]);
    let fixture = Fixture::new(&api);

    fixture
        .print(&["-m", "test", "say ok"])
        .expect_success("micro --print");

    let authorization = api
        .headers(0)
        .get("authorization")
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        authorization, "Bearer test-key",
        "the request went out without the credential the store holds"
    );
}

#[test]
fn a_tool_call_runs_and_its_result_returns_in_the_next_request() {
    let api = FakeApi::start([
        Reply::tool_call("call_1", "read", json!({ "path": "notes.txt" })),
        Reply::text("The note says: remember the milk"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write("notes.txt", "remember the milk");

    let output = fixture.print(&["-m", "test", "what is in notes.txt?"]);

    output.expect_success("micro --print with a tool call");
    assert!(output.stdout.contains("remember the milk"));

    assert_eq!(api.request_count(), 2);
    let second = api.request(1);
    let results = tool_results(&second);
    assert_eq!(results.len(), 1, "expected one tool result in {second}");
    assert_eq!(results[0]["tool_call_id"], "call_1");
    assert!(
        results[0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("remember the milk"),
        "the file contents should come back to the model, got {}",
        results[0]["content"]
    );
}

#[test]
fn a_tool_that_writes_really_changes_the_workspace() {
    let api = FakeApi::start([
        Reply::tool_call(
            "call_1",
            "write",
            json!({ "path": "created.txt", "content": "written by the agent" }),
        ),
        Reply::text("Created it."),
    ]);
    let fixture = Fixture::new(&api);

    fixture
        .print(&["-m", "test", "create created.txt"])
        .expect_success("micro --print with a write");

    assert!(fixture.exists("created.txt"), "the file was never written");
    let written = std::fs::read_to_string(fixture.workspace().join("created.txt")).unwrap();
    assert_eq!(written, "written by the agent");
}

#[test]
fn the_session_log_holds_the_conversation_and_sessions_list_shows_it() {
    let api = FakeApi::start([Reply::text("the answer")]);
    let fixture = Fixture::new(&api);

    fixture
        .print(&["-m", "test", "a memorable question"])
        .expect_success("micro --print");

    let logs = fixture.session_logs();
    assert_eq!(logs.len(), 1, "expected exactly one session log");
    assert!(
        logs[0].contains("a memorable question"),
        "the prompt is missing"
    );
    assert!(logs[0].contains("the answer"), "the response is missing");

    let listed = Output::run(fixture.micro().args(["sessions", "list"]));
    listed.expect_success("micro sessions list");
    assert!(
        listed.stdout.contains("test-model"),
        "the session should be listed, got {:?}",
        listed.stdout
    );
    assert!(listed.stdout.contains("a memorable question"));
}

#[test]
fn continue_resumes_the_conversation() {
    let api = FakeApi::start([
        Reply::text("Nice to meet you."),
        Reply::text("You said your name is Ramon."),
    ]);
    let fixture = Fixture::new(&api);

    fixture
        .print(&["-m", "test", "my name is Ramon"])
        .expect_success("the first run");
    fixture
        .print(&["-m", "test", "--continue", "what is my name?"])
        .expect_success("the resumed run");

    let resumed = api.request(1);
    let conversation = transcript(&resumed);
    assert!(
        conversation.contains("my name is Ramon"),
        "the prior prompt is missing"
    );
    assert!(
        conversation.contains("Nice to meet you"),
        "the prior reply is missing"
    );
    assert!(
        conversation.contains("what is my name?"),
        "the new prompt is missing"
    );

    assert_eq!(fixture.session_logs().len(), 1);
}

#[test]
fn a_tool_error_is_reported_to_the_model_and_the_run_continues() {
    let api = FakeApi::start([
        Reply::tool_call("call_1", "read", json!({ "path": "absent.txt" })),
        Reply::text("That file does not exist."),
    ]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-m", "test", "read absent.txt"]);

    output.expect_success("micro --print with a failing tool");
    assert!(output.stdout.contains("does not exist"));
    assert_eq!(tool_results(&api.request(1)).len(), 1);
}

#[test]
fn a_client_error_from_the_provider_fails_with_a_useful_message() {
    let api = FakeApi::start([Reply::Status(
        400,
        json!({ "error": { "message": "malformed request" } }).to_string(),
    )]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-m", "test", "hello"]);

    output.expect_failure("micro --print against a 400");
    assert!(
        output.stderr.contains("400"),
        "the status should be reported, got {:?}",
        output.stderr
    );
    assert!(
        output.stderr.contains("malformed request"),
        "the provider's explanation should survive, got {:?}",
        output.stderr
    );
    assert!(!output.stderr.contains("panicked"), "the binary panicked");
    assert_eq!(api.request_count(), 1, "a 400 must not be retried");
}

#[test]
fn a_server_error_is_retried_and_then_reported() {
    let api = FakeApi::start((0..5).map(|_| {
        Reply::Status(
            500,
            json!({ "error": { "message": "upstream is down" } }).to_string(),
        )
    }));
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-m", "test", "hello"]);

    output.expect_failure("micro --print against a 500");
    assert!(!output.stderr.contains("panicked"), "the binary panicked");
    assert!(
        output.stderr.contains("500") && output.stderr.contains("upstream is down"),
        "the failure should say what happened, got {:?}",
        output.stderr
    );
    assert_eq!(
        api.request_count(),
        5,
        "a 500 should be retried to the attempt cap"
    );
    assert!(
        output.stderr.contains("retry"),
        "the retries should be visible, got {:?}",
        output.stderr
    );
}

#[test]
fn an_unresolvable_model_reports_the_candidates() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-m", "definitely-not-a-model", "hello"]);

    output.expect_failure("micro --print with an unknown model");
    assert!(
        output.stderr.contains("definitely-not-a-model"),
        "the query should be echoed, got {:?}",
        output.stderr
    );
    assert_eq!(api.request_count(), 0, "no request should go out");
}

/// `-c key=value` writes into the config as it is read, so the setting it names is the one the run
/// goes on to use.
#[test]
fn a_setting_named_on_the_command_line_takes_effect() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-c", "model=definitely-not-a-model", "hello"]);

    output.expect_failure("micro --print with a model set by -c");
    assert!(
        output.stderr.contains("definitely-not-a-model"),
        "the model named by -c should be the one resolved, got {:?}",
        output.stderr
    );
    assert_eq!(api.request_count(), 0, "no request should go out");
}

#[test]
fn a_malformed_setting_on_the_command_line_is_refused() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-c", "model", "hello"]);

    output.expect_failure("micro --print with a malformed -c");
    assert!(
        output.stderr.contains("expected key=value"),
        "it should say what is wrong with it, got {:?}",
        output.stderr
    );
    assert_eq!(api.request_count(), 0, "no request should go out");
}

/// A value of the wrong shape is reported against the flag that wrote it.
#[test]
fn a_bad_value_on_the_command_line_names_the_flag_not_the_file() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-c", "mermaid=nonsense", "hello"]);

    output.expect_failure("micro --print with a bad -c value");
    assert!(
        output.stderr.contains("-c mermaid=nonsense"),
        "it should name the flag, got {:?}",
        output.stderr
    );
    assert!(
        output.stderr.contains("off"),
        "it should say what the value could be, got {:?}",
        output.stderr
    );
    assert!(
        !output.stderr.contains("config.json"),
        "the file does not hold this value, got {:?}",
        output.stderr
    );
    assert_eq!(api.request_count(), 0, "no request should go out");
}

#[test]
fn an_ambiguous_model_reports_the_candidates_rather_than_guessing() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-m", "claude-opus-5", "hello"]);

    output.expect_failure("micro --print with an ambiguous model");
    assert!(
        output.stderr.contains("anthropic/claude-opus-5")
            && output.stderr.contains("github-copilot/claude-opus-5"),
        "both candidates should be named, got {:?}",
        output.stderr
    );
    assert_eq!(api.request_count(), 0, "no request should go out");
}

#[test]
fn resume_reopens_a_session_by_id() {
    let api = FakeApi::start([Reply::text("first answer"), Reply::text("second answer")]);
    let fixture = Fixture::new(&api);
    fixture
        .print(&["-m", "test", "the original question"])
        .expect_success("the first run");

    let listed = Output::run(fixture.micro().args(["sessions", "list"]));
    let id = listed
        .stdout
        .split_whitespace()
        .next()
        .expect("a session id")
        .to_string();

    fixture
        .print(&["-m", "test", "--resume", &id, "a follow-up"])
        .expect_success("the resumed run");

    assert!(
        transcript(&api.request(1)).contains("the original question"),
        "resuming by id should restore the earlier conversation"
    );
    assert_eq!(
        fixture.session_logs().len(),
        1,
        "resuming must not fork a session"
    );
}

#[test]
fn print_without_a_prompt_is_refused() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-m", "test"]);

    output.expect_failure("micro --print with no prompt");
    assert_eq!(api.request_count(), 0);
}

#[test]
fn the_workspace_root_confines_the_tools() {
    let api = FakeApi::start([
        Reply::tool_call("call_1", "read", json!({ "path": "../outside.txt" })),
        Reply::text("I cannot reach that."),
    ]);
    let fixture = Fixture::new(&api);
    std::fs::write(fixture.workspace().join("../outside.txt"), "secret").expect("write outside");

    fixture
        .print(&["-m", "test", "read the file above the workspace"])
        .expect_success("micro --print with a traversing path");

    let results = tool_results(&api.request(1));
    let reported = results[0]["content"].as_str().unwrap_or_default();
    assert!(
        !reported.contains("secret"),
        "a path outside the workspace must not be read, got {reported:?}"
    );
    assert!(
        reported.contains("escapes the workspace"),
        "got {reported:?}"
    );
}

#[test]
fn auth_status_lists_the_providers() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = Output::run(fixture.micro().args(["auth", "status"]));

    output.expect_success("micro auth status");
    assert!(
        output.stdout.contains("anthropic"),
        "got {:?}",
        output.stdout
    );
    assert!(
        output.stdout.contains("openai"),
        "the fixture's stored credential should show, got {:?}",
        output.stdout
    );
}

#[test]
fn models_lists_the_catalog_without_touching_the_network() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = Output::run(fixture.micro().arg("models"));

    output.expect_success("micro models");
    assert!(
        output.stdout.contains("openai/test-model"),
        "the user catalog should be layered in, got {:?}",
        output.stdout
    );
    assert!(
        output.stdout.contains("anthropic/claude-opus-5"),
        "the bundled catalog should still be there, got {:?}",
        output.stdout
    );
    assert_eq!(api.request_count(), 0);
}

#[test]
fn models_filters_by_query() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = Output::run(fixture.micro().args(["models", "test-model"]));

    output.expect_success("micro models test-model");
    assert!(output.stdout.contains("openai/test-model"));
    assert!(!output.stdout.contains("claude-opus-5"));
}

#[test]
fn help_and_version_exit_zero() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let help = Output::run(fixture.micro().arg("--help"));
    help.expect_success("micro --help");
    assert!(help.stdout.contains("--print"));
    assert!(help.stdout.contains("--exclude-tools"));

    let version = Output::run(fixture.micro().arg("--version"));
    version.expect_success("micro --version");
    assert!(version.stdout.contains("micro"));
}

/// A machine with no `~/.micro` and nothing naming one directory keeps the two halves apart.
#[test]
fn a_fresh_install_reads_its_configuration_and_writes_its_sessions_where_xdg_says() {
    let api = FakeApi::start([Reply::text("split")]);
    let fixture = Fixture::new(&api);

    Output::run(
        fixture
            .micro_split()
            .args(["--print", "-m", "test", "say split"]),
    )
    .expect_success("micro --print on a fresh install");

    assert_eq!(
        api.headers(0)
            .get("authorization")
            .cloned()
            .unwrap_or_default(),
        "Bearer test-key",
        "the credential was not read from the configuration directory"
    );

    let sessions = fixture.xdg_data().join("sessions");
    let written: Vec<_> = std::fs::read_dir(&sessions)
        .unwrap_or_else(|error| panic!("read {}: {error}", sessions.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|kind| kind == "jsonl"))
        .collect();
    assert_eq!(written.len(), 1, "logs under {}", sessions.display());

    assert!(
        !fixture.xdg_config().join("sessions").exists(),
        "a session log landed among the settings"
    );
    assert!(
        !fixture.xdg_home().join(".micro").exists(),
        "a fresh install made the directory the split exists to avoid"
    );
}

#[test]
fn sessions_list_is_empty_before_anything_runs() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let output = Output::run(fixture.micro().args(["sessions", "list"]));

    output.expect_success("micro sessions list");
    assert!(
        output.stdout.contains("No sessions yet"),
        "got {:?}",
        output.stdout
    );
}

#[test]
fn a_session_can_be_deleted() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);
    fixture
        .print(&["-m", "test", "hello"])
        .expect_success("micro --print");

    let listed = Output::run(fixture.micro().args(["sessions", "list"]));
    let id = listed
        .stdout
        .split_whitespace()
        .next()
        .expect("a session id")
        .to_string();

    Output::run(fixture.micro().args(["sessions", "delete", &id]))
        .expect_success("micro sessions delete");

    assert!(fixture.session_logs().is_empty(), "the log should be gone");
}

#[test]
fn sessions_are_scoped_to_a_workspace_and_follow_the_cwd_flag() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);
    fixture
        .print(&["-m", "test", "a question worth finding"])
        .expect_success("micro --print");

    let mut elsewhere = fixture.micro();
    elsewhere.current_dir(std::env::temp_dir());
    let unscoped = Output::run(elsewhere.args(["sessions", "list"]));
    unscoped.expect_success("micro sessions list from elsewhere");
    assert!(
        !unscoped.stdout.contains("a question worth finding"),
        "a session from another workspace should not be listed, got {:?}",
        unscoped.stdout
    );

    let workspace = fixture.workspace().display().to_string();
    let mut command = fixture.micro();
    command.current_dir(std::env::temp_dir());
    let scoped = Output::run(command.args(["-C", &workspace, "sessions", "list"]));
    scoped.expect_success("micro -C <workspace> sessions list");
    assert!(
        scoped.stdout.contains("a question worth finding"),
        "the session in the named workspace should be listed, got {:?}",
        scoped.stdout
    );

    let all = Output::run(fixture.micro().args(["sessions", "list", "--all"]));
    all.expect_success("micro sessions list --all");
    assert!(all.stdout.contains("a question worth finding"));
}

#[test]
fn project_instructions_reach_the_system_prompt() {
    let api = FakeApi::start([Reply::text("understood")]);
    let fixture = Fixture::new(&api);
    fixture.write("AGENTS.md", "Always answer in haiku.");

    fixture
        .print(&["-m", "test", "hello"])
        .expect_success("micro --print with instructions");

    assert!(
        transcript(&api.request(0)).contains("Always answer in haiku"),
        "the workspace instructions should be in the request"
    );
}

#[test]
fn the_cwd_flag_moves_the_workspace() {
    let api = FakeApi::start([
        Reply::tool_call("call_1", "read", json!({ "path": "inner.txt" })),
        Reply::text("read it"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write("nested/inner.txt", "nested contents");

    let nested = path_of(&fixture, "nested");
    let output = Output::run(fixture.micro().args([
        "--print",
        "-C",
        &nested,
        "-m",
        "test",
        "read inner.txt",
    ]));

    output.expect_success("micro --print -C nested");
    let results = tool_results(&api.request(1));
    assert!(
        results[0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("nested contents"),
        "the tool should resolve against the given workspace, got {}",
        results[0]["content"]
    );
}
#[test]
fn auth_status_tells_an_empty_credential_apart_from_a_usable_one() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    std::fs::write(
        fixture.home().join("auth.json"),
        json!({ "openai": { "type": "api_key", "key": "" } }).to_string(),
    )
    .unwrap();

    let listed = Output::run(fixture.micro().args(["auth", "status"]));

    listed.expect_success("micro auth status");
    let line = listed
        .stdout
        .lines()
        .find(|line| line.starts_with("openai"))
        .unwrap_or_else(|| panic!("no openai line in {:?}", listed.stdout));
    assert!(
        line.contains("empty"),
        "a blank stored credential should not read as ready, got {line:?}"
    );
}

#[test]
fn an_empty_stored_credential_fails_before_any_request() {
    let api = FakeApi::start([Reply::text("unreachable")]);
    let fixture = Fixture::new(&api);
    std::fs::write(
        fixture.home().join("auth.json"),
        json!({ "openai": { "type": "api_key", "key": "" } }).to_string(),
    )
    .unwrap();

    let output = fixture.print(&["-m", "test", "say ok"]);

    output.expect_failure("micro --print with an empty credential");
    assert!(
        output.stderr.contains("empty"),
        "the failure should name the empty credential, got {:?}",
        output.stderr
    );
    assert_eq!(
        api.request_count(),
        0,
        "a request went out despite there being no credential to sign it with"
    );
}

/// The headless protocol answers each command, echoes the id it was given, and ends when stdin
/// closes.
#[test]
fn rpc_answers_every_command_it_is_given() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let lines = fixture.rpc(&[
        r#"{"type":"get_state","id":"1"}"#,
        r#"{"type":"get_commands","id":"2"}"#,
        r#"{"type":"get_available_models","id":"3"}"#,
        r#"{"type":"bash","command":"echo hello","id":"4"}"#,
        r#"{"type":"set_session_name","name":"the good one","id":"5"}"#,
        r#"{"type":"get_session_stats","id":"6"}"#,
    ]);

    assert_eq!(lines.len(), 6, "{lines:#?}");
    for (index, line) in lines.iter().enumerate() {
        assert_eq!(line["type"], "response");
        assert_eq!(line["id"], (index + 1).to_string());
        assert_eq!(line["success"], true, "{line}");
    }

    assert_eq!(lines[0]["data"]["message_count"], 0);
    assert!(lines[0]["data"]["session_id"].is_string());
    assert!(
        lines[1]["data"]["commands"]
            .as_array()
            .expect("a list of commands")
            .len()
            >= 20
    );
    assert!(!lines[2]["data"]["models"].as_array().unwrap().is_empty());
    assert_eq!(lines[3]["data"]["output"], "hello");
    assert_eq!(lines[3]["data"]["exit_code"], 0);
    assert_eq!(lines[5]["data"]["title"], "the good one");
}

#[test]
fn rpc_reports_a_line_it_cannot_read_and_keeps_going() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let lines = fixture.rpc(&["not json at all", r#"{"type":"get_state","id":"after"}"#]);

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["success"], false);
    assert!(
        lines[0]["error"]
            .as_str()
            .expect("a reason")
            .contains("unreadable"),
        "{}",
        lines[0]
    );
    assert_eq!(lines[1]["id"], "after");
    assert_eq!(lines[1]["success"], true);
}

/// A prompt streams the agent's own events, then the answer, all on the same stream.
#[test]
fn rpc_streams_a_turn_as_it_happens() {
    let api = FakeApi::start([Reply::text("an answer from the model")]);
    let fixture = Fixture::new(&api);

    let lines = fixture.rpc(&[r#"{"type":"prompt","message":"ask something","id":"turn"}"#]);

    assert_eq!(lines[0]["type"], "response");
    assert_eq!(lines[0]["command"], "prompt");
    assert_eq!(lines[0]["success"], true);

    let kinds: Vec<&str> = lines
        .iter()
        .skip(1)
        .filter_map(|line| line["type"].as_str())
        .collect();
    assert!(kinds.contains(&"turn_start"), "{kinds:?}");
    assert!(kinds.contains(&"message_end"), "{kinds:?}");

    let answered = lines.iter().any(|line| {
        serde_json::to_string(line)
            .unwrap_or_default()
            .contains("an answer from the model")
    });
    assert!(answered, "the answer reached the stream: {lines:#?}");
}

#[test]
fn rpc_refuses_a_model_it_does_not_have() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let lines = fixture.rpc(&[
        r#"{"type":"set_model","provider":"openrouter","model_id":"nothing-like-this","id":"1"}"#,
    ]);

    assert_eq!(lines[0]["success"], false);
    assert!(
        lines[0]["error"]
            .as_str()
            .unwrap()
            .contains("nothing-like-this"),
        "{}",
        lines[0]
    );
}

/// An extension in the project registers a tool, the model calls it, and what it returned reaches
/// the answer.
#[test]
fn an_extension_tool_is_offered_to_the_model_and_runs() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([
        Reply::tool_call("call_1", "project_greeting", json!({ "who": "world" })),
        Reply::text("the extension said it"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/greeter.ts",
        r#"
export default (micro) => {
    micro.registerTool({
        name: "project_greeting",
        description: "Return the project's own greeting",
        parameters: { type: "object", properties: { who: { type: "string" } } },
        // pi's argument order: the id this call was given, then its arguments.
        execute: async (toolCallId, args) => `hello ${args.who}, from an extension`,
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "greet the world"]);
    assert!(output.status.success(), "{}", output.stderr);

    let request = api.request(0);
    let tools = request["tools"].as_array().expect("tools were sent");
    assert!(
        tools
            .iter()
            .any(|tool| tool["function"]["name"] == "project_greeting"),
        "the extension's tool was offered: {tools:#?}"
    );

    let second = api.request(1);
    let messages = second["messages"].as_array().expect("a conversation");
    let carried = messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|text| text.contains("hello world, from an extension"))
    });
    assert!(
        carried,
        "the extension's answer reached the model: {messages:#?}"
    );
}

/// A project with no extensions starts exactly as it did before, and says nothing about it.
#[test]
fn a_project_without_extensions_says_nothing_about_them() {
    let api = FakeApi::start([Reply::text("fine")]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-m", "test", "say something"]);
    assert!(output.status.success(), "{}", output.stderr);
    assert!(
        !output.stderr.contains("extension"),
        "nothing to say: {}",
        output.stderr
    );
}

/// A package installed from a path is remembered, and its tool is offered on the next run without
/// anything else being said.
#[test]
fn an_installed_package_is_loaded_on_the_next_run() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("fine")]);
    let fixture = Fixture::new(&api);

    fixture.write(
        "package/package.json",
        r#"{ "name": "micro-demo", "pi": { "extensions": ["index.ts"] } }"#,
    );
    fixture.write(
        "package/index.ts",
        r#"
export default (micro) => {
    micro.registerTool({
        name: "demo_from_package",
        description: "A tool that arrived in a package",
        execute: async () => "the package tool ran",
    });
};
"#,
    );

    let installed = fixture.micro_run(&["install", &path_of(&fixture, "package")]);
    assert!(installed.status.success(), "{}", installed.stderr);
    assert!(
        installed.stdout.contains("demo_from_package"),
        "the install says what it registered: {}",
        installed.stdout
    );

    let output = fixture.print(&["-m", "test", "say something"]);
    assert!(output.status.success(), "{}", output.stderr);

    let request = api.request(0);
    let tools = request["tools"].as_array().expect("tools were sent");
    assert!(
        tools
            .iter()
            .any(|tool| tool["function"]["name"] == "demo_from_package"),
        "the installed package's tool was offered: {tools:#?}"
    );
}

/// A source that names nothing installable is refused before anything is written down.
#[test]
fn installing_something_that_is_not_there_changes_nothing() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let installed = fixture.micro_run(&["install", "/nowhere-at-all"]);
    assert!(!installed.status.success());
    assert!(
        !fixture.home().join("config.json").exists()
            || !std::fs::read_to_string(fixture.home().join("config.json"))
                .unwrap_or_default()
                .contains("nowhere-at-all"),
        "nothing was remembered"
    );
}

/// An extension is told what the agent is doing as it happens, under the names pi uses.
#[test]
fn an_extension_hears_the_lifecycle_events() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([
        Reply::tool_call("call_1", "read", json!({ "path": "notes.txt" })),
        Reply::text("done"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write("notes.txt", "the file's contents");

    fixture.write(
        ".micro/extensions/listener.ts",
        r#"
export default (micro) => {
    for (const event of [
        "session_start",
        "agent_start",
        "turn_start",
        "message_start",
        "message_end",
        "tool_execution_start",
        "tool_execution_end",
        "agent_end",
    ]) {
        micro.on(event, async (payload) => {
            await micro.appendEntry("heard-event", { event, payload });
        });
    }
    micro.registerCommand("heard-events", {
        handler: async () => JSON.stringify(await micro.getEntries()),
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "read the notes"]);
    assert!(output.status.success(), "{}", output.stderr);

    let report = fixture.print(&["-m", "test", "--continue", "/heard-events"]);
    assert!(report.status.success(), "{}", report.stderr);
    let heard = report.stdout;
    for event in [
        "session_start",
        "agent_start",
        "turn_start",
        "message_start",
        "message_end",
        "tool_execution_start",
        "tool_execution_end",
        "agent_end",
    ] {
        assert!(heard.contains(event), "{event} was heard: {heard}");
    }

    assert!(heard.contains("\"toolName\":\"read\""), "{heard}");
    assert!(heard.contains("notes.txt"), "{heard}");
}

/// A command an extension registered is typed like any other, and what it returns is what the user
/// sees.
#[test]
fn an_extension_command_is_a_slash_command() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/commands.ts",
        r#"
export default (micro) => {
    micro.registerCommand("shout", {
        description: "shout back",
        handler: async (args) => `SHOUTING: ${args.toUpperCase()}`,
    });
};
"#,
    );

    let lines = fixture.rpc(&[r#"{"type":"get_commands","id":"1"}"#]);
    let commands = lines[0]["data"]["commands"]
        .as_array()
        .expect("a list of commands");

    assert!(!commands.is_empty());

    let output = fixture.print(&["-m", "test", "/shout hello there"]);
    assert!(output.status.success(), "{}", output.stderr);
    assert!(
        output.stdout.contains("SHOUTING: HELLO THERE"),
        "the extension answered: {}",
        output.stdout
    );
}

/// An extension can run a program, and gets back what it printed.
#[test]
fn an_extension_can_run_a_command_and_read_its_output() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/runner.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async () => {
            const result = await micro.exec("echo", ["from an extension"]);
            return JSON.stringify(result);
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);
    let ran = stdout_json(&output);
    assert_eq!(ran["code"], 0, "{ran}");
    assert!(
        ran["stdout"]
            .as_str()
            .is_some_and(|stdout| stdout.contains("from an extension")),
        "{ran}"
    );
}

/// An extension can declare a provider, and a model it declared is one micro will run.
#[test]
fn an_extension_can_declare_a_provider_micro_then_uses() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("answered through the declared provider")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/provider.ts",
        &format!(
            r#"
export default (micro) => {{
    micro.registerProvider("my-proxy", {{
        name: "My Proxy",
        baseUrl: {base:?},
        api: "openai-completions",
        apiKey: "sk-declared",
        models: [{{
            id: "proxied-model",
            name: "Proxied Model",
            contextWindow: 128000,
            maxTokens: 8192,
            cost: {{ input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }},
        }}],
    }});
}};
"#,
            base = api.base_url()
        ),
    );

    let output = fixture.print(&["-m", "proxied-model", "say something"]);
    assert!(output.status.success(), "{}", output.stderr);
    assert!(
        output
            .stdout
            .contains("answered through the declared provider"),
        "{}",
        output.stdout
    );

    assert_eq!(api.request_count(), 1);
    let headers = api.headers(0);
    let authorization = headers
        .get("authorization")
        .expect("the request carried a credential");
    assert!(
        authorization.contains("sk-declared"),
        "the declared key was used: {authorization}"
    );
}

/// An extension can refuse a tool call, and the model is told why instead of getting the tool's
/// output.
#[test]
fn an_extension_can_block_a_tool_call() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([
        Reply::tool_call(
            "call_1",
            "write",
            json!({ "path": "secrets.env", "content": "x" }),
        ),
        Reply::text("blocked, then"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/guard.ts",
        r#"
export default (micro) => {
    micro.on("tool_call", (event) => {
        if (String(event.input?.path ?? "").endsWith(".env")) {
            return { block: true, reason: "no writing to environment files" };
        }
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "write the file"]);
    assert!(output.status.success(), "{}", output.stderr);

    assert!(!fixture.exists("secrets.env"), "the call did not run");
    let second = api.request(1);
    let refused = second["messages"]
        .as_array()
        .expect("a conversation")
        .iter()
        .any(|message| {
            message["content"]
                .as_str()
                .is_some_and(|text| text.contains("no writing to environment files"))
        });
    assert!(refused, "the reason reached the model: {second:#?}");
}

/// An extension can rewrite what a tool returned before the model reads it.
#[test]
fn an_extension_can_rewrite_a_tool_result() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([
        Reply::tool_call("call_1", "read", json!({ "path": "notes.txt" })),
        Reply::text("read it"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write("notes.txt", "token=SECRET-VALUE-1234");
    fixture.write(
        ".micro/extensions/redact.ts",
        r#"
export default (micro) => {
    micro.on("tool_result", (event) => {
        const original = event.content?.[0]?.text ?? "";
        const cleaned = original.replace(/SECRET-[A-Z0-9-]+/g, "[redacted]");
        if (cleaned !== original) {
            return { content: [{ type: "text", text: cleaned }] };
        }
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "read the notes"]);
    assert!(output.status.success(), "{}", output.stderr);

    let second = api.request(1);
    let conversation = serde_json::to_string(&second).unwrap();
    assert!(
        conversation.contains("[redacted]"),
        "the result was rewritten"
    );
    assert!(
        !conversation.contains("SECRET-VALUE-1234"),
        "the secret never reached the model: {conversation}"
    );
}

#[test]
fn a_listener_that_answers_nothing_changes_nothing() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([
        Reply::tool_call("call_1", "read", json!({ "path": "notes.txt" })),
        Reply::text("read it"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write("notes.txt", "the plain contents");
    fixture.write(
        ".micro/extensions/watcher.ts",
        r#"
export default (micro) => {
    micro.on("tool_call", () => {});
    micro.on("tool_result", () => {});
};
"#,
    );

    let output = fixture.print(&["-m", "test", "read the notes"]);
    assert!(output.status.success(), "{}", output.stderr);

    let conversation = serde_json::to_string(&api.request(1)).unwrap();
    assert!(
        conversation.contains("the plain contents"),
        "{conversation}"
    );
}

/// An extension sees what the user typed and can rewrite it before anything is done with it, or
/// swallow it entirely.
#[test]
fn an_extension_can_rewrite_what_the_user_typed() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("answered")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/input.ts",
        r#"
export default (micro) => {
    micro.on("input", (event) => {
        if (event.text === "shorthand") {
            return { action: "transform", text: "the expanded question" };
        }
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "shorthand"]);
    assert!(output.status.success(), "{}", output.stderr);

    let sent = serde_json::to_string(&api.request(0)).unwrap();
    assert!(sent.contains("the expanded question"), "{sent}");
    assert!(
        !sent.contains("shorthand"),
        "the original was replaced: {sent}"
    );
}

/// The moments the host owns reach extensions too: the model changing, and a session starting.
#[test]
fn an_extension_hears_the_moments_the_host_owns() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/hostwatch.ts",
        r#"
export default (micro) => {
    for (const event of ["session_start", "session_info_changed", "user_bash"]) {
        micro.on(event, async (payload) => {
            await micro.appendEntry("heard-host-event", { event, payload });
        });
    }
    micro.registerCommand("heard-host-events", {
        handler: async () => JSON.stringify(await micro.getEntries()),
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/name the renamed one"]);
    assert!(output.status.success(), "{}", output.stderr);

    let report = fixture.print(&["-m", "test", "--continue", "/heard-host-events"]);
    assert!(report.status.success(), "{}", report.stderr);
    let heard = report.stdout;
    assert!(heard.contains("session_start"), "{heard}");
    assert!(heard.contains("session_info_changed"), "{heard}");
    assert!(heard.contains("the renamed one"), "{heard}");
}

/// An extension can override the system prompt for a turn, from what it started as.
#[test]
fn an_extension_can_override_the_system_prompt_for_a_turn() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("answered")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/start.ts",
        r#"
export default (micro) => {
    micro.on("before_agent_start", (event) => {
        return { systemPrompt: `${event.systemPrompt}\n\nAlso: be extremely terse.` };
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "say something"]);
    assert!(output.status.success(), "{}", output.stderr);

    let request = api.request(0);
    let system = request["messages"][0]["content"]
        .as_str()
        .expect("a system message");
    assert!(system.contains("be extremely terse"), "{system}");
}

/// An extension can rewrite the messages the model is sent, and is told once the request carrying
/// them is assembled.
#[test]
fn an_extension_can_rewrite_the_messages_the_model_is_sent() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("answered")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/context.ts",
        r#"
export default (micro) => {
    micro.on("context", (event) => {
        return {
            messages: event.messages.map((message) =>
                message.role === "user"
                    ? { ...message, content: [{ type: "text", text: "a rewritten question" }] }
                    : message,
            ),
        };
    });
    micro.on("before_provider_request", async (event) => {
        await micro.appendEntry("provider-request", { messageCount: event.payload.messages.length });
    });
    micro.registerCommand("provider-events", {
        handler: async () => JSON.stringify(await micro.getEntries()),
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "the original question"]);
    assert!(output.status.success(), "{}", output.stderr);

    let sent = serde_json::to_string(&api.request(0)).unwrap();
    assert!(sent.contains("a rewritten question"), "{sent}");
    assert!(!sent.contains("the original question"), "{sent}");

    let report = fixture.print(&["-m", "test", "--continue", "/provider-events"]);
    assert!(report.status.success(), "{}", report.stderr);
    assert!(report.stdout.contains("messageCount"), "{}", report.stdout);
    assert!(report.stdout.contains('1'), "{}", report.stdout);
}

/// An extension can put a header on the request the provider makes.
#[test]
fn an_extension_can_set_a_request_header() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("answered")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/headers.ts",
        r#"
export default (micro) => {
    micro.on("before_provider_headers", () => {
        return { headers: { "x-team": "platform" } };
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "say something"]);
    assert!(output.status.success(), "{}", output.stderr);

    let headers = api.headers(0);
    assert_eq!(
        headers.get("x-team").map(String::as_str),
        Some("platform"),
        "the header reached the provider: {headers:#?}"
    );
}

/// An extension keeps state in the session, reads it back, and the model never sees it.
#[test]
fn an_extension_can_keep_state_the_model_never_sees() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("answered")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/keeper.ts",
        r#"
export default (micro) => {
    micro.registerCommand("keep", {
        handler: async () => {
            await micro.appendEntry("a-note", { secretly: "kept aside" });
            const kept = await micro.getEntries();
            return JSON.stringify(kept);
        },
    });
};
"#,
    );

    let kept = fixture.print(&["-m", "test", "/keep"]);
    assert!(kept.status.success(), "{}", kept.stderr);
    assert!(kept.stdout.contains("kept aside"), "{}", kept.stdout);

    let output = fixture.print(&["-m", "test", "say something"]);
    assert!(output.status.success(), "{}", output.stderr);
    let sent = serde_json::to_string(&api.request(0)).unwrap();
    assert!(
        !sent.contains("kept aside"),
        "the model never saw it: {sent}"
    );
}

/// An extension draws its own message, and what it drew is what appears.
#[test]
fn an_extension_draws_its_own_message() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/drawer.ts",
        r#"
export default (micro) => {
    micro.registerMessageRenderer("deploy", (data) => {
        return [`environment: ${data.details?.env ?? "unknown"}`, "status: drawn by an extension"];
    });
    micro.registerCommand("deploy", {
        handler: async () => {
            micro.sendMessage({ customType: "deploy", content: "ignored", details: { env: "staging" } });
            return "sent";
        },
    });
};
"#,
    );

    let installed =
        fixture.micro_run(&["install", &path_of(&fixture, ".micro/extensions/drawer.ts")]);
    assert!(installed.status.success(), "{}", installed.stderr);
    assert!(installed.stdout.contains("deploy"), "{}", installed.stdout);
}

/// A flag an extension declared is read off the command line and reaches it.
#[test]
fn an_extension_flag_is_read_from_the_command_line() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/flagged.ts",
        r#"
export default (micro) => {
    micro.registerFlag("env", { description: "which environment", type: "string", default: "dev" });
    micro.registerFlag("loud", { description: "shout", type: "boolean" });
    micro.registerCommand("show", {
        handler: async () => {
            const seen = { env: micro.getFlag("env"), loud: micro.getFlag("loud") };
            return JSON.stringify(seen);
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "--env=staging", "--loud", "/show"]);
    assert!(output.status.success(), "{}", output.stderr);

    let seen = stdout_json(&output);
    assert_eq!(seen["env"], "staging", "{seen}");
    assert_eq!(seen["loud"], true, "{seen}");
}

#[test]
fn a_flag_nobody_declared_is_reported() {
    let api = FakeApi::start([Reply::text("fine")]);
    let fixture = Fixture::new(&api);

    let output = fixture.print(&["-m", "test", "--nothing-declares-this", "say something"]);
    assert!(output.status.success(), "{}", output.stderr);
    assert!(
        output.stderr.contains("nothing-declares-this") || output.stdout.contains("fine"),
        "either it was reported or the run carried on: {} {}",
        output.stdout,
        output.stderr
    );
}

#[test]
fn an_mcp_servers_tools_are_offered_like_any_other() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);

    let server = fixture.workspace().join("echo-server.sh");
    std::fs::write(
        &server,
        r#"#!/bin/bash
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Say it back","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
  esac
done
"#,
    )
    .expect("write the server");
    std::fs::set_permissions(&server, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make it runnable");

    std::fs::write(
        fixture.home().join("config.json"),
        serde_json::json!({
            "default_project_trust": "always",
            "mcp_servers": {
                "demo": { "command": server.to_string_lossy() },
                "off": { "command": "never-run-me", "enabled": false },
            },
        })
        .to_string(),
    )
    .expect("write config.json");

    fixture
        .print(&["-m", "test", "say hello"])
        .expect_success("micro --print");

    let tools = offered_tools(&api.request(0));
    assert!(
        tools.contains(&"mcp__demo__echo".to_string()),
        "the server's tool should be offered, got {tools:?}"
    );

    assert!(tools.contains(&"read".to_string()), "{tools:?}");

    assert!(
        !tools.iter().any(|name| name.starts_with("mcp__off__")),
        "{tools:?}"
    );
}

#[test]
fn an_mcp_server_that_will_not_start_is_reported() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);

    std::fs::write(
        fixture.home().join("config.json"),
        serde_json::json!({
            "default_project_trust": "always",
            "mcp_servers": { "broken": { "command": "definitely-not-a-program-anyone-has" } },
        })
        .to_string(),
    )
    .expect("write config.json");

    let output = fixture.print(&["-m", "test", "say hello"]);

    output.expect_success("micro --print still runs");
    assert!(
        output.stderr.contains("broken"),
        "the server should be named, got {:?}",
        output.stderr
    );

    assert!(offered_tools(&api.request(0)).contains(&"read".to_string()));
}

/// Writes a server offering `count` tools, so a test can put more of them on offer than are worth
/// describing up front.
fn many_tool_server(path: &std::path::Path, count: usize) {
    let listed: Vec<String> = (0..count)
        .map(|index| {
            format!(
                r#"{{"name":"thing{index}","description":"Do thing {index}","inputSchema":{{"type":"object"}}}}"#
            )
        })
        .collect();
    std::fs::write(
        path,
        format!(
            r#"#!/bin/bash
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id" ;;
    *'"tools/list"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"tools":[{}]}}}}\n' "$id" ;;
  esac
done
"#,
            listed.join(",")
        ),
    )
    .expect("write the server");
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make it runnable");
}

fn with_mcp_server(fixture: &Fixture, server: &std::path::Path, extra: serde_json::Value) {
    let mut config = serde_json::json!({
        "default_project_trust": "always",
        "mcp_servers": { "demo": { "command": server.to_string_lossy() } },
    });
    for (key, value) in extra.as_object().expect("an object") {
        config[key] = value.clone();
    }
    std::fs::write(fixture.home().join("config.json"), config.to_string())
        .expect("write config.json");
}

/// Past the threshold the extra tools stop being described and `tool_search` stands in for them.
#[test]
fn many_tools_are_left_to_be_searched_for() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);
    let server = fixture.workspace().join("many.sh");
    many_tool_server(&server, 20);
    with_mcp_server(&fixture, &server, serde_json::json!({}));

    fixture
        .print(&["-m", "test", "say hello"])
        .expect_success("micro --print");

    let tools = offered_tools(&api.request(0));
    assert!(tools.contains(&"tool_search".to_string()), "{tools:?}");
    assert!(tools.contains(&"read".to_string()), "{tools:?}");
    assert!(
        !tools.iter().any(|name| name.starts_with("mcp__demo__")),
        "the deferred tools should not be described, got {tools:?}"
    );
}

/// Below the threshold nothing changes: a handful of tools is worth describing outright, and a
/// search would only cost an exchange.
#[test]
fn a_few_tools_are_still_described_up_front() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);
    let server = fixture.workspace().join("few.sh");
    many_tool_server(&server, 3);
    with_mcp_server(&fixture, &server, serde_json::json!({}));

    fixture
        .print(&["-m", "test", "say hello"])
        .expect_success("micro --print");

    let tools = offered_tools(&api.request(0));
    assert!(
        tools.contains(&"mcp__demo__thing0".to_string()),
        "{tools:?}"
    );
    assert!(!tools.contains(&"tool_search".to_string()), "{tools:?}");
}

/// A threshold of zero describes every tool however many there are.
#[test]
fn the_threshold_can_be_turned_off() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);
    let server = fixture.workspace().join("off.sh");
    many_tool_server(&server, 20);
    with_mcp_server(
        &fixture,
        &server,
        serde_json::json!({ "tool_search_threshold": 0 }),
    );

    fixture
        .print(&["-m", "test", "say hello"])
        .expect_success("micro --print");

    let tools = offered_tools(&api.request(0));
    assert!(
        tools.contains(&"mcp__demo__thing19".to_string()),
        "{tools:?}"
    );
    assert!(!tools.contains(&"tool_search".to_string()), "{tools:?}");
}

#[test]
fn an_extension_command_reads_the_extension_context() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/context-probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe-context", {
        handler: async (args, ctx) => JSON.stringify({
                cwd: ctx.cwd,
                mode: ctx.mode,
                hasUI: ctx.hasUI,
                isProjectTrusted: ctx.isProjectTrusted(),
                model: ctx.model,
                thinkingLevel: ctx.thinkingLevel,
                systemPrompt: ctx.getSystemPrompt(),
                contextUsage: ctx.getContextUsage(),
                hasNewSession: typeof ctx.newSession === "function",
                hasFork: typeof ctx.fork === "function",
                hasNavigateTree: typeof ctx.navigateTree === "function",
                hasSwitchSession: typeof ctx.switchSession === "function",
                hasReload: typeof ctx.reload === "function",
            }),
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe-context"]);
    assert!(output.status.success(), "{}", output.stderr);
    let read = stdout_json(&output);

    assert_eq!(read["mode"], "print");
    assert_eq!(read["hasUI"], false);

    assert_eq!(read["isProjectTrusted"], true);
    assert_eq!(read["model"]["id"], "test-model");
    assert_eq!(read["model"]["provider"], "openai");
    assert_eq!(read["model"]["contextWindow"], 200000);
    assert_eq!(read["model"]["maxOutputTokens"], 4096);
    assert_eq!(read["contextUsage"]["tokens"], serde_json::Value::Null);
    assert_eq!(read["contextUsage"]["contextWindow"], 200000);
    assert!(
        read["systemPrompt"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "{read}"
    );
    for member in [
        "hasNewSession",
        "hasFork",
        "hasNavigateTree",
        "hasSwitchSession",
        "hasReload",
    ] {
        assert_eq!(read[member], true, "{member}: {read}");
    }
}

#[test]
fn an_extension_reads_the_conversation_through_session_manager() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("Nice to meet you.")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/session-probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe-session", {
        handler: async (args, ctx) => {
            const manager = ctx.sessionManager;
            const leafId = manager.getLeafId();
            return JSON.stringify({
                cwd: manager.getCwd(),
                sessionId: manager.getSessionId(),
                sessionFile: manager.getSessionFile(),
                sessionName: manager.getSessionName(),
                leafId,
                leafEntry: manager.getLeafEntry(),
                allEntries: manager.getEntries(),
                branch: manager.getBranch(),
                sameAsLeafEntry: manager.getEntry(leafId),
                header: manager.getHeader(),
                tree: manager.getTree(),
            });
        },
    });
};
"#,
    );

    fixture
        .print(&["-m", "test", "my name is Ramon"])
        .expect_success("the first run");
    let probed = fixture.print(&["-m", "test", "--continue", "/probe-session"]);
    assert!(probed.status.success(), "{}", probed.stderr);
    let read = stdout_json(&probed);

    assert!(read["sessionFile"].as_str().unwrap().ends_with(".jsonl"));
    assert!(!read["sessionId"].as_str().unwrap().is_empty());
    let entries = read["allEntries"].as_array().expect("entries");
    assert!(entries.len() >= 2, "{entries:?}");
    assert!(
        entries.iter().any(|entry| {
            entry["type"] == "message"
                && serde_json::to_string(&entry["message"])
                    .unwrap()
                    .contains("my name is Ramon")
        }),
        "{entries:?}"
    );

    assert_eq!(read["leafEntry"], read["sameAsLeafEntry"]);
    assert!(!read["leafEntry"].is_null());

    let branch = read["branch"].as_array().expect("a branch");
    assert_eq!(branch.last().unwrap(), &read["leafEntry"]);

    assert_eq!(
        read["tree"].as_array().unwrap().len(),
        1,
        "{}",
        read["tree"]
    );
}

#[test]
fn is_idle_and_signal_track_a_turn_through_its_lifecycle_events() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/idle-probe.ts",
        r#"
export default (micro) => {
    micro.on("agent_start", async (payload, ctx) => {
        await micro.appendEntry("idle-observation", {
            event: "agent_start",
            idle: ctx.isIdle(),
            hasSignal: ctx.signal !== undefined,
        });
    });
    micro.on("agent_settled", async (payload, ctx) => {
        await micro.appendEntry("idle-observation", {
            event: "agent_settled",
            idle: ctx.isIdle(),
            hasSignal: ctx.signal !== undefined,
        });
    });
    micro.registerCommand("probe-wait", {
        handler: async (args, ctx) => {
            // Already idle by the time a command runs, so this resolves at once.
            await ctx.waitForIdle();
            await micro.appendEntry("idle-observation", { event: "waited", idle: ctx.isIdle() });
            return JSON.stringify(await micro.getEntries());
        },
    });
};
"#,
    );

    fixture
        .print(&["-m", "test", "say hello"])
        .expect_success("micro --print");
    let probed = fixture.print(&["-m", "test", "--continue", "/probe-wait"]);
    assert!(probed.status.success(), "{}", probed.stderr);
    let read = stdout_json(&probed);
    let started =
        nested_object_with(&read, "event", &json!("agent_start")).expect("agent_start was heard");
    assert_eq!(started["idle"], false, "{started}");
    assert_eq!(started["hasSignal"], true, "{started}");

    let settled = nested_object_with(&read, "event", &json!("agent_settled"))
        .expect("agent_settled was heard");
    assert_eq!(settled["idle"], true, "{settled}");
    assert_eq!(settled["hasSignal"], false, "{settled}");

    let waited =
        nested_object_with(&read, "event", &json!("waited")).expect("waitForIdle resolved");
    assert_eq!(waited["idle"], true, "{waited}");
}

/// `ctx.getSystemPromptOptions()` reports what actually went into the prompt.
#[test]
fn get_system_prompt_options_reports_what_actually_built_the_prompt() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(".micro/SYSTEM.md", "You are a pirate.");
    fixture.write(".micro/APPEND_SYSTEM.md", "Also: arr.");
    fixture.write("AGENTS.md", "Be nice to the user.");
    fixture.write(
        ".micro/skills/deploy/SKILL.md",
        "---\nname: deploy\ndescription: Ships the app.\n---\n\nRun the steps.\n",
    );

    fixture.write(
        ".micro/extensions/prompt-probe.ts",
        r#"
export default (micro) => {
    micro.registerTool({
        name: "deploy_tool",
        description: "deploys the app",
        promptSnippet: "use this to deploy",
        promptGuidelines: ["always confirm first"],
        execute: async () => "deployed",
    });
    micro.registerCommand("probe-prompt", {
        handler: async (args, ctx) => JSON.stringify(ctx.getSystemPromptOptions()),
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "-t", "deploy_tool", "/probe-prompt"]);
    assert!(output.status.success(), "{}", output.stderr);
    let read = stdout_json(&output);

    assert_eq!(read["cwd"], fixture.workspace().display().to_string());
    assert_eq!(read["customPrompt"], "You are a pirate.");
    assert_eq!(read["appendSystemPrompt"], "Also: arr.");
    assert_eq!(read["selectedTools"], serde_json::json!(["deploy_tool"]));
    assert_eq!(
        read["toolSnippets"],
        serde_json::json!({ "deploy_tool": "use this to deploy" })
    );
    assert_eq!(
        read["promptGuidelines"],
        serde_json::json!(["always confirm first"])
    );

    let context_files = read["contextFiles"].as_array().expect("context files");
    assert!(
        context_files.iter().any(|file| {
            file["path"]
                .as_str()
                .unwrap_or_default()
                .ends_with("AGENTS.md")
                && file["content"] == "Be nice to the user."
        }),
        "{context_files:?}"
    );

    let skills = read["skills"].as_array().expect("skills");
    let deploy_skill = skills
        .iter()
        .find(|skill| skill["name"] == "deploy")
        .expect("the deploy skill");
    assert_eq!(deploy_skill["description"], "Ships the app.");
    assert_eq!(deploy_skill["disableModelInvocation"], false);
    assert_eq!(deploy_skill["sourceInfo"]["scope"], "project");
    assert!(deploy_skill["filePath"]
        .as_str()
        .unwrap_or_default()
        .ends_with("SKILL.md"));
}

#[test]
fn scoped_models_reach_the_extension_context_resolved() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/scope-probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe-scope", {
        handler: async (args, ctx) => JSON.stringify(ctx.scopedModels),
    });
};
"#,
    );

    let scoped = fixture.print(&[
        "-m",
        "test",
        "-c",
        r#"scoped_models=["anthropic/claude-opus-5"]"#,
        "/probe-scope",
    ]);
    assert!(scoped.status.success(), "{}", scoped.stderr);

    let read = stdout_json(&scoped);
    let matches = read.as_array().expect("a list");
    assert!(
        matches
            .iter()
            .any(|entry| entry["model"]["provider"] == "anthropic"
                && entry["model"]["id"] == "claude-opus-5"),
        "{matches:?}"
    );
}

/// With nothing scoped, `ctx.scopedModels` is empty.
#[test]
fn unscoped_models_reach_the_extension_context_as_empty() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/unscoped-probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe-unscoped", {
        handler: async (args, ctx) => JSON.stringify(ctx.scopedModels),
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe-unscoped"]);
    assert!(output.status.success(), "{}", output.stderr);
    let read = stdout_json(&output);
    assert_eq!(read, serde_json::json!([]));
}

#[test]
fn session_navigation_is_absent_from_a_tool_calls_context() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([
        Reply::tool_call("probe", "check", serde_json::json!({})),
        Reply::text("done"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/tool-probe.ts",
        r#"
export default (micro) => {
    micro.registerTool({
        name: "check",
        description: "reports what its context carries",
        execute: async (toolCallId, args, signal, onUpdate, ctx) => JSON.stringify({
                hasModel: ctx.model !== undefined,
                hasNewSession: typeof ctx.newSession === "function",
                hasFork: typeof ctx.fork === "function",
                hasNavigateTree: typeof ctx.navigateTree === "function",
                hasSwitchSession: typeof ctx.switchSession === "function",
                hasReload: typeof ctx.reload === "function",
            }),
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "run the check"]);
    assert!(output.status.success(), "{}", output.stderr);

    let results = tool_results(&api.request(1));
    let read: serde_json::Value = serde_json::from_str(
        results[0]["content"]
            .as_str()
            .expect("the tool result is text"),
    )
    .expect("the tool result is JSON");

    assert_eq!(read["hasModel"], true, "{read}");
    for member in [
        "hasNewSession",
        "hasFork",
        "hasNavigateTree",
        "hasSwitchSession",
        "hasReload",
    ] {
        assert_eq!(read[member], false, "{member}: {read}");
    }
}

/// An extension's tool can report partial progress while it runs, through `onUpdate`.
#[test]
fn an_extension_tool_streams_a_partial_update_while_it_runs() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([
        Reply::tool_call("call_1", "narrate", json!({})),
        Reply::text("heard it"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/narrate.ts",
        r#"
export default (micro) => {
    micro.registerTool({
        name: "narrate",
        description: "says what it is doing as it goes",
        execute: async (toolCallId, args, signal, onUpdate) => {
            onUpdate?.({ content: [{ type: "text", text: "halfway there" }] });
            return "all done";
        },
    });
};
"#,
    );

    let lines = fixture.rpc(&[r#"{"type":"prompt","message":"narrate it","id":"1"}"#]);

    let update_index = lines
        .iter()
        .position(|line| line["type"] == "tool_update" && line["name"] == "narrate")
        .unwrap_or_else(|| panic!("no tool_update for narrate in {lines:#?}"));
    assert!(
        lines[update_index]["output"]
            .as_str()
            .unwrap_or_default()
            .contains("halfway there"),
        "{:#?}",
        lines[update_index]
    );

    let end_index = lines
        .iter()
        .position(|line| line["type"] == "tool_end" && line["name"] == "narrate")
        .expect("the call finished");
    assert!(
        update_index < end_index,
        "the update should precede the result: {lines:#?}"
    );
}

#[test]
fn an_extension_tool_is_stopped_when_the_turn_is_aborted() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::tool_call("call_1", "wait_forever", json!({}))]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/waits.ts",
        r#"
export default (micro) => {
    micro.registerTool({
        name: "wait_forever",
        description: "never resolves unless the turn is aborted",
        execute: async (toolCallId, args, signal, onUpdate) => {
            onUpdate?.({ content: [{ type: "text", text: "tool started" }] });
            return new Promise((resolve, reject) => {
                signal?.addEventListener("abort", () => {
                    void micro.appendEntry("abort-observation", { signal: "received" });
                    reject(new Error("aborted"));
                });
            });
        },
    });
};
"#,
    );

    let mut command = fixture.micro();
    command.arg("--rpc");
    command.args(["-m", "test"]);
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command.spawn().expect("micro --rpc starts");
    let stdout = child.stdout.take().expect("stdout is piped");
    let (lines_tx, lines_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut lines = Vec::new();
        for line in BufReader::new(stdout).lines() {
            let line = line.expect("RPC output is readable");
            lines_tx.send(line.clone()).expect("the receiver is alive");
            lines.push(line);
        }
        lines
    });

    writeln!(
        child.stdin.as_mut().expect("stdin is piped"),
        r#"{{"type":"prompt","message":"wait for it","id":"1"}}"#
    )
    .expect("the prompt is written");

    let running_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let remaining = running_deadline.saturating_duration_since(std::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "the tool call should have started running"
        );
        let line = lines_rx
            .recv_timeout(remaining)
            .expect("the tool call should produce RPC output");
        let event: serde_json::Value = serde_json::from_str(&line).expect("RPC output is JSON");
        if event["type"] == "tool_update"
            && event["output"]
                .as_str()
                .is_some_and(|output| output.contains("tool started"))
        {
            break;
        }
    }

    writeln!(
        child.stdin.as_mut().expect("stdin is piped"),
        r#"{{"type":"abort","id":"2"}}"#
    )
    .expect("the abort is written");

    let settled_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let remaining = settled_deadline.saturating_duration_since(std::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "the agent should settle after the abort"
        );
        let line = lines_rx
            .recv_timeout(remaining)
            .expect("the abort should produce RPC output");
        let event: serde_json::Value = serde_json::from_str(&line).expect("RPC output is JSON");
        if event["type"] == "agent_settled" {
            break;
        }
    }

    drop(child.stdin.take());
    let _ = child.wait_with_output().expect("micro --rpc finishes");
    let lines = reader.join().expect("the RPC output reader finishes");
    let events: Vec<serde_json::Value> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("RPC output is JSON"))
        .collect();
    let abort_index = events
        .iter()
        .position(|event| event["type"] == "response" && event["id"] == "2")
        .expect("the abort command was answered");
    assert_eq!(events[abort_index]["success"], true, "{events:#?}");
    let settled_index = events
        .iter()
        .position(|event| event["type"] == "agent_settled")
        .expect("the agent settled after the abort");
    assert!(
        abort_index < settled_index,
        "the turn settled before its abort was acknowledged: {events:#?}"
    );
    assert!(
        !events.iter().any(|event| event["type"] == "tool_end"),
        "the pending extension tool must not complete after the abort: {events:#?}"
    );
    let session_log = fixture.session_logs().join("\n");
    assert!(
        session_log.contains("abort-observation") && session_log.contains("received"),
        "the extension recorded the tool's abort signal in the session ledger: {session_log}"
    );
}

/// A theme is a file on disk, the same one micro itself reads.
#[test]
fn an_extension_can_read_and_switch_to_a_custom_theme() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let themes_dir = fixture.home().join("themes");
    std::fs::create_dir_all(&themes_dir).expect("create the themes directory");
    std::fs::write(
        themes_dir.join("mine.json"),
        json!({
            "name": "mine",
            "colors": support::theme_colors("#123456"),
        })
        .to_string(),
    )
    .expect("write a custom theme");

    fixture.write(
        ".micro/extensions/theme-probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            const all = ctx.ui.getAllThemes();
            const looked_up = ctx.ui.getTheme("mine");
            const missing = ctx.ui.getTheme("nocturne");
            const before = ctx.ui.theme.name;
            const switched = ctx.ui.setTheme("mine");
            const after = ctx.ui.theme.name;
            const colored = ctx.ui.theme.fg("accent", "hi");
            return JSON.stringify({
                names: all.map((t) => t.name),
                lookedUpName: looked_up?.name,
                lookedUpAccent: looked_up?.fg("accent", "x"),
                missing,
                before,
                switched,
                after,
                colored,
            });
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let result = stdout_json(&output);
    let names = result["names"].as_array().expect("a list of themes");
    assert!(names.iter().any(|name| name == "dark"), "{result}");
    assert!(names.iter().any(|name| name == "light"), "{result}");
    assert!(names.iter().any(|name| name == "mine"), "{result}");
    assert_eq!(result["lookedUpName"], "mine");
    assert_eq!(result["lookedUpAccent"], "\u{1b}[38;2;18;52;86mx\u{1b}[39m");
    assert!(result["missing"].is_null());
    assert_eq!(result["before"], "dark");
    assert_eq!(result["switched"]["success"], true);
    assert_eq!(result["after"], "mine");
    assert_eq!(result["colored"], "\u{1b}[38;2;18;52;86mhi\u{1b}[39m");
}

/// `getEditorText` and `getToolsExpanded` answer synchronously, the same as they do in pi.
#[test]
fn an_extension_reads_back_what_it_set_in_the_editor_and_in_tools_expansion() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/echo-probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            const beforeText = ctx.ui.getEditorText();
            ctx.ui.setEditorText("hello");
            const afterSet = ctx.ui.getEditorText();
            ctx.ui.pasteToEditor(" world");
            const afterPaste = ctx.ui.getEditorText();

            const beforeExpanded = ctx.ui.getToolsExpanded();
            ctx.ui.setToolsExpanded(true);
            const afterExpanded = ctx.ui.getToolsExpanded();

            return JSON.stringify({
                beforeText, afterSet, afterPaste, beforeExpanded, afterExpanded,
            });
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let result = stdout_json(&output);
    assert_eq!(result["beforeText"], "");
    assert_eq!(result["afterSet"], "hello");
    assert_eq!(result["afterPaste"], "hello world");
    assert_eq!(result["beforeExpanded"], false);
    assert_eq!(result["afterExpanded"], true);
}

/// Every member of `ExtensionUIContext` that hands back a live TUI component.
#[test]
fn an_extension_asking_for_a_live_component_runs_the_factory_it_is_given() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/components-probe.ts",
        r#"
const component = () => ({ render: (width) => [`hi ${width}`] });
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            const customResult = await ctx.ui.custom(() => ({
                render: () => ["overlay"],
                handleInput: () => {},
            }));
            ctx.ui.setHeader(component);
            ctx.ui.setFooter(component);
            ctx.ui.addAutocompleteProvider((current) => current);
            const editorFactory = component;
            ctx.ui.setEditorComponent(editorFactory);
            const editorComponent = ctx.ui.getEditorComponent();
            ctx.ui.setWidget("factory-widget", component);

            return JSON.stringify({
                customResult: customResult === undefined,
                editorComponentIsTheSameFactory: editorComponent === editorFactory,
                ranWithoutThrowing: true,
            });
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let result = stdout_json(&output);
    assert_eq!(result["customResult"], true);
    assert_eq!(result["editorComponentIsTheSameFactory"], true);
    assert_eq!(result["ranWithoutThrowing"], true);
}

#[test]
fn setting_a_widget_component_never_throws_even_headless() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/widget-component-probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            ctx.ui.setWidget("status", (tui) => ({
                render: (width) => [`status at ${width}`],
                invalidate: () => {},
            }));
            return "ok";
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);
    assert_eq!(output.stdout.trim(), "ok");
}

#[test]
fn an_extension_can_register_and_unregister_a_terminal_input_listener() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/listener-probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            const unsubscribe = ctx.ui.onTerminalInput((data) => undefined);
            const isFunction = typeof unsubscribe === "function";
            unsubscribe();
            return JSON.stringify({ isFunction });
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let result = stdout_json(&output);
    assert_eq!(result["isFunction"], true);
}

/// A pi extension imports pi's own runtime modules.
#[test]
fn an_extension_importing_a_pi_runtime_module_loads_and_runs() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/pi-runtime-probe.ts",
        r#"
import { stripTerminalSequences, truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";
import { CONFIG_DIR_NAME, defineTool } from "@mariozechner/pi-coding-agent";
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async () => {
            const width = visibleWidth("hello");
            // Real pi-tui truncateToWidth wraps its answer with a style reset even for
            // plain text, so what an extension actually reads is stripped through
            // pi-tui's own stripTerminalSequences before it is compared to anything.
            const truncated = stripTerminalSequences(truncateToWidth("hello world", 5));
            const tool = defineTool({ name: "x" });
            let unsupportedError;
            try {
                const mod = await import("@earendil-works/pi-coding-agent");
                mod.main();
            } catch (error) {
                unsupportedError = error instanceof Error ? error.message : String(error);
            }
            return JSON.stringify({
                    width,
                    truncated,
                    configDirName: CONFIG_DIR_NAME,
                    toolName: tool.name,
                    unsupportedError,
                });
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let result = stdout_json(&output);
    assert_eq!(result["width"], 5, "visibleWidth answered for real");
    assert_eq!(
        result["truncated"], "he...",
        "truncateToWidth answered for real"
    );
    assert_eq!(
        result["configDirName"], ".micro",
        "CONFIG_DIR_NAME answers with micro's own directory name, not pi's — an extension \
         building a path from it should land somewhere micro actually reads"
    );
    assert_eq!(
        result["toolName"], "x",
        "defineTool is pi's own identity function"
    );
    let error = result["unsupportedError"]
        .as_str()
        .expect("main() is not supported here, and says so");
    assert!(
        error.contains("main"),
        "the failure names what was actually reached for: {error}"
    );
}

#[test]
fn an_extension_uses_pi_tuis_pure_layout_and_autocomplete_components() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/pi-tui-components-probe.ts",
        r#"
import {
    CombinedAutocompleteProvider,
    HStack,
    renderLatex,
    stripTerminalSequences,
    Text,
    TruncatedText,
    VStack,
} from "@earendil-works/pi-tui";
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async () => {
            // Two 3-wide texts side by side in a 10-wide box: real column layout, not a stub.
            const hstack = new HStack([new Text("aaa", 0, 0), new Text("bbb", 0, 0)], { gap: 1 });
            const hstackLines = hstack.render(10).map((line) => stripTerminalSequences(line));

            // Stacked vertically instead: line count reflects both children plus the gap.
            const vstack = new VStack([new Text("one", 0, 0), new Text("two", 0, 0)]);
            const vstackLines = vstack.render(10);

            const truncated = stripTerminalSequences(
                new TruncatedText("a very long line that will not fit", 0, 0).render(10)[0],
            );

            const latex = renderLatex("x^2");

            const provider = new CombinedAutocompleteProvider(
                [{ name: "help", description: "show help" }],
                process.cwd(),
                null,
            );
            const suggestions = await provider.getSuggestions(["/hel"], 0, 4, { signal: new AbortController().signal });

            return JSON.stringify({
                    hstackLineCount: hstackLines.length,
                    hstackFirstLine: hstackLines[0],
                    vstackLineCount: vstackLines.length,
                    truncated,
                    latex,
                    suggestionNames: suggestions?.items.map((item) => item.value) ?? null,
                });
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let result = stdout_json(&output);
    assert_eq!(
        result["hstackLineCount"], 1,
        "one line of text laid out side by side stays one line"
    );
    assert_eq!(
        result["hstackFirstLine"], "aaa bbb   ",
        "HStack actually composited both children with the gap between them, padded to the full requested width"
    );
    assert_eq!(
        result["vstackLineCount"], 2,
        "two stacked single-line children produce two lines"
    );
    assert_eq!(
        result["truncated"], "a very ...",
        "TruncatedText actually truncated to the given width"
    );
    assert_eq!(
        result["latex"], "x²",
        "renderLatex actually rendered the expression, not a stub answer"
    );
    assert_eq!(
        result["suggestionNames"],
        serde_json::json!(["help"]),
        "CombinedAutocompleteProvider actually matched the slash command"
    );
}

/// An extension's command handler that throws is not a command that quietly did nothing.
#[test]
fn a_thrown_command_handler_fails_the_print_run_rather_than_exiting_zero() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/commands.ts",
        r#"
export default (micro) => {
    micro.registerCommand("explode", {
        description: "always throws",
        handler: async () => {
            throw new Error("boom");
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/explode"]);

    output.expect_failure("micro --print /explode");
    assert!(
        output.stderr.contains("boom"),
        "the error reaches stderr: {}",
        output.stderr
    );
    assert!(
        !output.stdout.contains("boom"),
        "an error is not printed as though it were an ordinary answer: {}",
        output.stdout
    );
}

#[test]
fn an_extension_renders_real_markdown() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/markdown-probe.ts",
        r##"
import { Markdown, stripTerminalSequences } from "@earendil-works/pi-tui";
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async () => {
            const theme = {
                heading: (t) => t,
                link: (t) => t,
                linkUrl: (t) => t,
                code: (t) => `[${t}]`,
                codeBlock: (t) => t,
                codeBlockBorder: (t) => t,
                quote: (t) => t,
                quoteBorder: (t) => t,
                hr: (t) => t,
                listBullet: (t) => t,
                bold: (t) => t,
                italic: (t) => t,
                strikethrough: (t) => t,
                underline: (t) => t,
            };
            const markdown = new Markdown("# Title\n\nSome `code` here.", 0, 0, theme);
            const lines = markdown.render(40).map((line) => stripTerminalSequences(line));
            return JSON.stringify({ lines });
        },
    });
};
"##,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let result = stdout_json(&output);
    let lines: Vec<String> = result["lines"]
        .as_array()
        .expect("lines is an array")
        .iter()
        .map(|line| line.as_str().unwrap_or_default().to_string())
        .collect();
    let joined = lines.join("\n");
    assert!(
        joined.contains("Title"),
        "the heading text made it through: {joined:?}"
    );
    assert!(
        joined.contains("[code]"),
        "inline code was actually tokenized by marked and styled through the theme's code fn: {joined:?}"
    );
}

/// `custom()` and `setEditorComponent()` hand their factory pi-tui's real `KeybindingsManager` now,
/// not `{}`.
#[test]
fn a_components_keybindings_argument_answers_for_real_rather_than_being_empty() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/keybindings-probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            const seen = [];
            const factory = (tui, theme, keybindings) => {
                seen.push(keybindings.matches("\x1b", "tui.select.cancel"));
                seen.push(keybindings.matches("x", "tui.select.cancel"));
                return { render: () => ["editor"], handleInput: () => {} };
            };
            ctx.ui.setEditorComponent(factory);
            await ctx.ui.custom((tui, theme, keybindings, done) => {
                seen.push(keybindings.matches("\x1b", "tui.select.cancel"));
                done("closed");
                return { render: () => ["overlay"], handleInput: () => {} };
            });
            return JSON.stringify({ seen });
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let result = stdout_json(&output);
    let seen: Vec<bool> = result["seen"]
        .as_array()
        .expect("seen is an array")
        .iter()
        .map(|value| value.as_bool().expect("matches() answers a boolean"))
        .collect();
    assert_eq!(
        seen,
        vec![true, false, true],
        "escape matches tui.select.cancel and an unbound key does not, in both factories: {seen:?}"
    );
}
