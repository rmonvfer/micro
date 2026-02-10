//! End-to-end tests of the compiled `micro` binary against a fake provider served from
//! the test process. Nothing here reaches the network or the caller's own configuration.

mod support;

use serde_json::json;
use support::offered_tools;
use support::path_of;
use support::tool_results;
use support::transcript;
use support::FakeApi;
use support::Fixture;
use support::Output;
use support::Reply;

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

    // The workspace tools are offered, so the model can actually do something.
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

    // Two requests: the one that asked for the tool, and the one carrying its result.
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

    // Writing is asked about under the default mode, so this run allows it outright.
    fixture
        .print(&["-m", "test", "--approve", "workspace", "create created.txt"])
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

    // The resumed run's request carries the earlier exchange, not just the new prompt.
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

    // Both runs share one session rather than starting a second.
    assert_eq!(fixture.session_logs().len(), 1);
}

#[test]
fn cautious_refuses_a_shell_command_rather_than_running_it() {
    let marker = "should-not-exist.txt";
    let api = FakeApi::start([
        Reply::tool_call(
            "call_1",
            "bash",
            json!({ "command": format!("touch {marker}") }),
        ),
        Reply::text("I could not run that."),
    ]);
    let fixture = Fixture::new(&api);

    // Default mode, and stdin is closed, so there is nobody to approve the command.
    let output = fixture.print(&["-m", "test", "create a file with the shell"]);

    output.expect_success("micro --print with a refused command");
    assert!(
        !fixture.exists(marker),
        "the command ran despite not being approved"
    );

    let results = tool_results(&api.request(1));
    assert_eq!(results.len(), 1);
    let reported = results[0]["content"].as_str().unwrap_or_default();
    assert!(
        reported.contains("not approved") || reported.contains("approval"),
        "the model should be told why the command did not run, got {reported:?}"
    );
}

#[test]
fn an_explicit_policy_rule_lets_a_command_through() {
    let api = FakeApi::start([
        Reply::tool_call("call_1", "bash", json!({ "command": "echo approved" })),
        Reply::text("It printed approved."),
    ]);
    let fixture = Fixture::new(&api);
    std::fs::write(
        fixture.home().join("policy.json"),
        json!({ "mode": "cautious", "rules": { "bash:echo": "allow" } }).to_string(),
    )
    .expect("write policy.json");

    fixture
        .print(&["-m", "test", "echo something"])
        .expect_success("micro --print with an allowing rule");

    let results = tool_results(&api.request(1));
    assert_eq!(results.len(), 1);
    let reported = results[0]["content"].as_str().unwrap_or_default();
    assert!(
        reported.contains("approved"),
        "the command should have run, got {reported:?}"
    );
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
    // Five attempts, four of them retries, so the script holds five failures.
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

#[test]
fn an_ambiguous_model_reports_the_candidates_rather_than_guessing() {
    // `claude-opus-5` is served by more than one provider in the bundled catalog, and
    // silently picking one would bill the wrong account.
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
    assert!(help.stdout.contains("--approve"));

    let version = Output::run(fixture.micro().arg("--version"));
    version.expect_success("micro --version");
    assert!(version.stdout.contains("micro"));
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

    // From an unrelated directory the session belongs to somewhere else.
    let mut elsewhere = fixture.micro();
    elsewhere.current_dir(std::env::temp_dir());
    let unscoped = Output::run(elsewhere.args(["sessions", "list"]));
    unscoped.expect_success("micro sessions list from elsewhere");
    assert!(
        !unscoped.stdout.contains("a question worth finding"),
        "a session from another workspace should not be listed, got {:?}",
        unscoped.stdout
    );

    // Naming that workspace with `-C` brings it back, from anywhere.
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

    // `--all` ignores the scoping entirely.
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
