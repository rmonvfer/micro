//! End-to-end tests of the compiled `micro` binary against a fake provider served from
//! the test process. Nothing here reaches the network or the caller's own configuration.

mod support;

use serde_json::json;
use micro_extensions::which_bun;
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

/// The headless protocol answers each command, echoes the id it was given, and ends when
/// stdin closes.
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

/// A line that is not a command is reported rather than ignored, and the stream carries on.
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

    // The command is acknowledged before the turn runs, so a caller knows it started.
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

/// A model the catalog does not have is refused by name rather than silently kept.
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

/// An extension in the project registers a tool, the model calls it, and what it returned
/// reaches the answer — through a real Bun process, with no configuration anywhere.
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
        execute: async (args) => `hello ${args.who}, from an extension`,
    });
};
"#,
    );

    // Approval is given up front: an extension's tool is third-party code, so without
    // this the policy asks, and nothing is there to answer.
    let output = fixture.print(&["-m", "test", "--approve", "unrestricted", "greet the world"]);
    assert!(output.status.success(), "{}", output.stderr);

    // The model was offered the extension's tool by name.
    let request = api.request(0);
    let tools = request["tools"].as_array().expect("tools were sent");
    assert!(
        tools.iter().any(|tool| tool["function"]["name"] == "project_greeting"),
        "the extension's tool was offered: {tools:#?}"
    );

    // And what the extension returned went back to the model as the result.
    let second = api.request(1);
    let messages = second["messages"].as_array().expect("a conversation");
    let carried = messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|text| text.contains("hello world, from an extension"))
    });
    assert!(carried, "the extension's answer reached the model: {messages:#?}");
}

/// An extension's tool goes through the same policy as everything built in, so an
/// unattended run refuses it rather than running someone else's code unasked.
#[test]
fn an_extension_tool_is_gated_like_every_other_tool() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([
        Reply::tool_call("call_1", "project_greeting", json!({ "who": "world" })),
        Reply::text("it was refused"),
    ]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/greeter.ts",
        r#"
export default (micro) => {
    micro.registerTool({
        name: "project_greeting",
        description: "Return the project's own greeting",
        execute: async () => "this should not have run",
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "greet the world"]);
    assert!(output.status.success(), "{}", output.stderr);

    let messages = api.request(1);
    let refused = messages["messages"]
        .as_array()
        .expect("a conversation")
        .iter()
        .any(|message| {
            message["content"]
                .as_str()
                .is_some_and(|text| text.contains("Refused by the workspace policy"))
        });
    assert!(refused, "the call was refused: {messages:#?}");
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

/// A package installed from a path is remembered, and its tool is offered on the next run
/// without anything else being said.
#[test]
fn an_installed_package_is_loaded_on_the_next_run() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("fine")]);
    let fixture = Fixture::new(&api);

    // A package the way one arrives from npm: a manifest naming its entry point.
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

    // Nothing else is configured: the next run finds it through the settings alone.
    let output = fixture.print(&["-m", "test", "say something"]);
    assert!(output.status.success(), "{}", output.stderr);

    let request = api.request(0);
    let tools = request["tools"].as_array().expect("tools were sent");
    assert!(
        tools.iter().any(|tool| tool["function"]["name"] == "demo_from_package"),
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

/// An extension is told what the agent is doing as it happens, under the names ohm uses.
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

    // The extension writes down every event it hears, so the test can read them back.
    let log = fixture.workspace().join("events.log");
    fixture.write(
        ".micro/extensions/listener.ts",
        &format!(
            r#"
import {{ appendFileSync }} from "node:fs";
const log = {log:?};
export default (micro) => {{
    for (const event of [
        "session_start",
        "agent_start",
        "turn_start",
        "message_start",
        "message_end",
        "tool_execution_start",
        "tool_execution_end",
        "agent_end",
    ]) {{
        micro.on(event, (payload) => {{
            appendFileSync(log, `${{event}} ${{JSON.stringify(payload).slice(0, 120)}}\n`);
        }});
    }}
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "--approve", "unrestricted", "read the notes"]);
    assert!(output.status.success(), "{}", output.stderr);

    let heard = std::fs::read_to_string(&log).unwrap_or_default();
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

    // And the events carry what happened, not just that it happened.
    assert!(heard.contains("\"toolName\":\"read\""), "{heard}");
    assert!(heard.contains("notes.txt"), "{heard}");
}

/// A command an extension registered is typed like any other, and what it returns is what
/// the user sees.
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
    // The built-in list is what `get_commands` reports; the extension's own is reached by
    // typing it, which is what the next assertion covers.
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
    let log = fixture.workspace().join("exec.log");
    fixture.write(
        ".micro/extensions/runner.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async () => {{
            const result = await micro.exec("echo", ["from an extension"]);
            writeFileSync({log:?}, JSON.stringify(result));
            return `exit ${{result.code}}`;
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);
    assert!(output.stdout.contains("exit 0"), "{}", output.stdout);

    let ran = std::fs::read_to_string(&log).expect("the extension wrote what it got");
    assert!(ran.contains("from an extension"), "{ran}");
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
        output.stdout.contains("answered through the declared provider"),
        "{}",
        output.stdout
    );

    // The request went to the declared endpoint with the declared credential.
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

/// An extension can refuse a tool call, and the model is told why instead of getting the
/// tool's output.
#[test]
fn an_extension_can_block_a_tool_call() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([
        Reply::tool_call("call_1", "write", json!({ "path": "secrets.env", "content": "x" })),
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

    let output = fixture.print(&["-m", "test", "--approve", "unrestricted", "write the file"]);
    assert!(output.status.success(), "{}", output.stderr);

    // The file was never written, and the model was told why.
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
        const cleaned = String(event.result ?? "").replace(/SECRET-[A-Z0-9-]+/g, "[redacted]");
        if (cleaned !== event.result) {
            return { content: cleaned };
        }
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "--approve", "unrestricted", "read the notes"]);
    assert!(output.status.success(), "{}", output.stderr);

    let second = api.request(1);
    let conversation = serde_json::to_string(&second).unwrap();
    assert!(conversation.contains("[redacted]"), "the result was rewritten");
    assert!(
        !conversation.contains("SECRET-VALUE-1234"),
        "the secret never reached the model: {conversation}"
    );
}

/// An extension that only listens changes nothing, which is what keeps a watcher from
/// accidentally intercepting.
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

    let output = fixture.print(&["-m", "test", "--approve", "unrestricted", "read the notes"]);
    assert!(output.status.success(), "{}", output.stderr);

    let conversation = serde_json::to_string(&api.request(1)).unwrap();
    assert!(conversation.contains("the plain contents"), "{conversation}");
}

/// An extension sees what the user typed and can rewrite it before anything is done with
/// it, or swallow it entirely.
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
            return { text: "the expanded question" };
        }
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "shorthand"]);
    assert!(output.status.success(), "{}", output.stderr);

    let sent = serde_json::to_string(&api.request(0)).unwrap();
    assert!(sent.contains("the expanded question"), "{sent}");
    assert!(!sent.contains("shorthand"), "the original was replaced: {sent}");
}

/// The moments the host owns reach extensions too: the model changing, and a session
/// starting.
#[test]
fn an_extension_hears_the_moments_the_host_owns() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("host-events.log");
    fixture.write(
        ".micro/extensions/hostwatch.ts",
        &format!(
            r#"
import {{ appendFileSync }} from "node:fs";
export default (micro) => {{
    for (const event of ["session_start", "session_info_changed", "user_bash"]) {{
        micro.on(event, (payload) => {{
            appendFileSync({log:?}, `${{event}} ${{JSON.stringify(payload)}}\n`);
        }});
    }}
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/name the renamed one"]);
    assert!(output.status.success(), "{}", output.stderr);

    let heard = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(heard.contains("session_start"), "{heard}");
    assert!(heard.contains("session_info_changed"), "{heard}");
    assert!(heard.contains("the renamed one"), "{heard}");
}
