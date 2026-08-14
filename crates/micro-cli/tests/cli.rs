//! End-to-end tests of the compiled `micro` binary against a fake provider served from
//! the test process. Nothing here reaches the network or the caller's own configuration.

mod support;

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

/// `-c key=value` writes into the config as it is read, so the setting it names is the
/// one the run goes on to use.
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

/// A mistyped setting is a mistake to correct rather than one to run past: falling back
/// to the stored settings would do the work with something other than what was asked for.
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

/// A value of the wrong shape is reported against the flag that wrote it. Naming the
/// config file would point at somewhere the bad value is not.
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
    assert!(help.stdout.contains("--exclude-tools"));

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
        // pi's argument order: the id this call was given, then its arguments.
        execute: async (toolCallId, args) => `hello ${args.who}, from an extension`,
    });
};
"#,
    );

    // Approval is given up front: an extension's tool is third-party code, so without
    // this the policy asks, and nothing is there to answer.
    let output = fixture.print(&["-m", "test", "greet the world"]);
    assert!(output.status.success(), "{}", output.stderr);

    // The model was offered the extension's tool by name.
    let request = api.request(0);
    let tools = request["tools"].as_array().expect("tools were sent");
    assert!(
        tools
            .iter()
            .any(|tool| tool["function"]["name"] == "project_greeting"),
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

    let output = fixture.print(&["-m", "test", "read the notes"]);
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
        output
            .stdout
            .contains("answered through the declared provider"),
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

    let output = fixture.print(&["-m", "test", "read the notes"]);
    assert!(output.status.success(), "{}", output.stderr);

    let conversation = serde_json::to_string(&api.request(1)).unwrap();
    assert!(
        conversation.contains("the plain contents"),
        "{conversation}"
    );
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

    // The system prompt the model received carries the extension's addition.
    let request = api.request(0);
    let system = request["messages"][0]["content"]
        .as_str()
        .expect("a system message");
    assert!(system.contains("be extremely terse"), "{system}");
}

/// An extension can rewrite the messages the model is sent, and is told once the request
/// carrying them is assembled.
#[test]
fn an_extension_can_rewrite_the_messages_the_model_is_sent() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("answered")]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("provider.log");
    fixture.write(
        ".micro/extensions/context.ts",
        &format!(
            r#"
import {{ appendFileSync }} from "node:fs";
export default (micro) => {{
    micro.on("context", (event) => {{
        return {{
            messages: event.messages.map((message) =>
                message.role === "user"
                    ? {{ ...message, content: [{{ type: "text", text: "a rewritten question" }}] }}
                    : message,
            ),
        }};
    }});
    micro.on("before_provider_request", (event) => {{
        appendFileSync({log:?}, `request ${{event.payload.messages.length}}\n`);
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "the original question"]);
    assert!(output.status.success(), "{}", output.stderr);

    let sent = serde_json::to_string(&api.request(0)).unwrap();
    assert!(sent.contains("a rewritten question"), "{sent}");
    assert!(!sent.contains("the original question"), "{sent}");

    let seen = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(seen.contains("request 1"), "{seen}");
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
    let log = fixture.workspace().join("kept.log");
    fixture.write(
        ".micro/extensions/keeper.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("keep", {{
        handler: async () => {{
            await micro.appendEntry("a-note", {{ secretly: "kept aside" }});
            const kept = await micro.getEntries();
            writeFileSync({log:?}, JSON.stringify(kept));
            return `kept ${{kept.length}}`;
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let kept = fixture.print(&["-m", "test", "/keep"]);
    assert!(kept.status.success(), "{}", kept.stderr);
    assert!(kept.stdout.contains("kept 1"), "{}", kept.stdout);

    let read_back = std::fs::read_to_string(&log).expect("the extension read it back");
    assert!(read_back.contains("kept aside"), "{read_back}");

    // The next run sends the conversation, and what was kept aside is not in it.
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

    // The renderer is registered, and micro knows which types it draws.
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
    let log = fixture.workspace().join("flag.log");
    fixture.write(
        ".micro/extensions/flagged.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerFlag("env", {{ description: "which environment", type: "string", default: "dev" }});
    micro.registerFlag("loud", {{ description: "shout", type: "boolean" }});
    micro.registerCommand("show", {{
        handler: async () => {{
            const seen = {{ env: micro.getFlag("env"), loud: micro.getFlag("loud") }};
            writeFileSync({log:?}, JSON.stringify(seen));
            return JSON.stringify(seen);
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "--env=staging", "--loud", "/show"]);
    assert!(output.status.success(), "{}", output.stderr);

    let seen = std::fs::read_to_string(&log).expect("the extension read its flags");
    assert!(seen.contains("\"env\":\"staging\""), "{seen}");
    assert!(seen.contains("\"loud\":true"), "{seen}");
}

/// A flag nobody declared is said out loud rather than ignored.
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

/// A tool another program provides is offered to the model beside micro's own, under a
/// name that says where it came from.
#[test]
fn an_mcp_servers_tools_are_offered_like_any_other() {
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);

    // A server is any program that speaks the protocol on its stdin and stdout, so the
    // test uses one rather than standing in for it.
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
    // micro's own are still there beside it.
    assert!(tools.contains(&"read".to_string()), "{tools:?}");
    // A server that is turned off is not started, so nothing of its is offered.
    assert!(
        !tools.iter().any(|name| name.starts_with("mcp__off__")),
        "{tools:?}"
    );
}

/// A server that cannot start is named rather than passed over, and costs only its own
/// tools.
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
    // The run went ahead on the tools that did load.
    assert!(offered_tools(&api.request(0)).contains(&"read".to_string()));
}

/// Writes a server offering `count` tools, so a test can put more of them on offer than
/// are worth describing up front.
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

/// Past the threshold the extra tools stop being described and `tool_search` stands in
/// for them. The built-in ones are still there, since deferring those would cost a search
/// before the model could read a file.
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

/// Below the threshold nothing changes: a handful of tools is worth describing outright,
/// and a search would only cost an exchange.
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

/// A command handler's second argument is pi's `ExtensionCommandContext`: where the run
/// is, what it is running, and the handful of things a command is allowed to do to it.
#[test]
fn an_extension_command_reads_the_extension_context() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("context.log");
    fixture.write(
        ".micro/extensions/context-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe-context", {{
        handler: async (args, ctx) => {{
            writeFileSync({log:?}, JSON.stringify({{
                cwd: ctx.cwd,
                mode: ctx.mode,
                hasUI: ctx.hasUI,
                isProjectTrusted: ctx.isProjectTrusted(),
                model: ctx.model,
                thinkingLevel: ctx.thinkingLevel,
                systemPrompt: ctx.getSystemPrompt(),
                contextUsage: ctx.getContextUsage(),
                // Present only on a command's context, not a tool's or an event
                // handler's — the same restriction pi places on
                // `ExtensionCommandContext` versus the plain `ExtensionContext`.
                hasNewSession: typeof ctx.newSession === "function",
                hasFork: typeof ctx.fork === "function",
                hasNavigateTree: typeof ctx.navigateTree === "function",
                hasSwitchSession: typeof ctx.switchSession === "function",
                hasReload: typeof ctx.reload === "function",
            }}));
            return "probed";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe-context"]);
    assert!(output.status.success(), "{}", output.stderr);
    assert!(output.stdout.contains("probed"), "{}", output.stdout);

    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&log).expect("the extension wrote a log"))
            .expect("the log is JSON");

    assert_eq!(read["mode"], "print");
    assert_eq!(read["hasUI"], false);
    // Baked into every `Fixture`: `default_project_trust: "always"`.
    assert_eq!(read["isProjectTrusted"], true);
    assert_eq!(read["model"]["id"], "test-model");
    assert_eq!(read["model"]["provider"], "openai");
    assert_eq!(read["model"]["contextWindow"], 200000);
    assert_eq!(read["model"]["maxOutputTokens"], 4096);
    assert_eq!(read["contextUsage"]["tokens"], serde_json::Value::Null);
    assert_eq!(read["contextUsage"]["contextWindow"], 200000);
    assert!(
        read["systemPrompt"].as_str().is_some_and(|text| !text.is_empty()),
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

/// `ctx.sessionManager` reads back a conversation that already happened, entirely from
/// what `get_context` carried over in one round trip — no further asking.
#[test]
fn an_extension_reads_the_conversation_through_session_manager() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("Nice to meet you.")]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("session-manager.log");
    fixture.write(
        ".micro/extensions/session-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe-session", {{
        handler: async (args, ctx) => {{
            const manager = ctx.sessionManager;
            const leafId = manager.getLeafId();
            writeFileSync({log:?}, JSON.stringify({{
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
            }}));
            return "read";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    fixture
        .print(&["-m", "test", "my name is Ramon"])
        .expect_success("the first run");
    let probed = fixture.print(&["-m", "test", "--continue", "/probe-session"]);
    assert!(probed.status.success(), "{}", probed.stderr);
    assert!(probed.stdout.contains("read"), "{}", probed.stdout);

    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&log).expect("the extension wrote a log"))
            .expect("the log is JSON");

    assert!(read["sessionFile"].as_str().unwrap().ends_with(".jsonl"));
    assert!(!read["sessionId"].as_str().unwrap().is_empty());
    let entries = read["allEntries"].as_array().expect("entries");
    assert!(entries.len() >= 2, "{entries:?}"); // the prompt, and the reply
    assert!(
        entries.iter().any(|entry| {
            entry["type"] == "message"
                && serde_json::to_string(&entry["message"])
                    .unwrap()
                    .contains("my name is Ramon")
        }),
        "{entries:?}"
    );
    // The leaf, looked up two ways, is the same entry.
    assert_eq!(read["leafEntry"], read["sameAsLeafEntry"]);
    assert!(!read["leafEntry"].is_null());
    // getBranch() with no argument walks from the leaf, root to head.
    let branch = read["branch"].as_array().expect("a branch");
    assert_eq!(branch.last().unwrap(), &read["leafEntry"]);
    // The tree covers the same ground as the flat entry list.
    assert_eq!(read["tree"].as_array().unwrap().len(), 1, "{}", read["tree"]);
}

/// `ctx.isIdle()` and `ctx.signal` track a real turn from the lifecycle events micro
/// already forwards: busy with a signal to hand out while `agent_start` is heard, idle
/// with none by the time `agent_settled` is.
#[test]
fn is_idle_and_signal_track_a_turn_through_its_lifecycle_events() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("done")]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("idle.log");
    fixture.write(
        ".micro/extensions/idle-probe.ts",
        &format!(
            r#"
import {{ appendFileSync }} from "node:fs";
const log = {log:?};
export default (micro) => {{
    micro.on("agent_start", (payload, ctx) => {{
        appendFileSync(log, JSON.stringify({{
            event: "agent_start",
            idle: ctx.isIdle(),
            hasSignal: ctx.signal !== undefined,
        }}) + "\n");
    }});
    micro.on("agent_settled", (payload, ctx) => {{
        appendFileSync(log, JSON.stringify({{
            event: "agent_settled",
            idle: ctx.isIdle(),
            hasSignal: ctx.signal !== undefined,
        }}) + "\n");
    }});
    micro.registerCommand("probe-wait", {{
        handler: async (args, ctx) => {{
            // Already idle by the time a command runs, so this resolves at once.
            await ctx.waitForIdle();
            appendFileSync(log, JSON.stringify({{ event: "waited", idle: ctx.isIdle() }}) + "\n");
            return "waited";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    fixture
        .print(&["-m", "test", "say hello"])
        .expect_success("micro --print");
    let probed = fixture.print(&["-m", "test", "--continue", "/probe-wait"]);
    assert!(probed.status.success(), "{}", probed.stderr);
    assert!(probed.stdout.contains("waited"), "{}", probed.stdout);

    let read = std::fs::read_to_string(&log).expect("the extension logged something");
    let entries: Vec<serde_json::Value> = read
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect();

    let started = entries
        .iter()
        .find(|entry| entry["event"] == "agent_start")
        .expect("agent_start was heard");
    assert_eq!(started["idle"], false, "{started}");
    assert_eq!(started["hasSignal"], true, "{started}");

    let settled = entries
        .iter()
        .find(|entry| entry["event"] == "agent_settled")
        .expect("agent_settled was heard");
    assert_eq!(settled["idle"], true, "{settled}");
    assert_eq!(settled["hasSignal"], false, "{settled}");

    let waited = entries
        .iter()
        .find(|entry| entry["event"] == "waited")
        .expect("waitForIdle resolved");
    assert_eq!(waited["idle"], true, "{waited}");
}

/// `ctx.getSystemPromptOptions()` reports what actually went into the prompt — a custom
/// SYSTEM.md, an APPEND_SYSTEM.md, a context file with its own content, a loaded skill,
/// and a tool's own snippet — read back through the whole wire, not asserted against
/// Rust's own state directly.
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

    let log = fixture.workspace().join("options.log");
    fixture.write(
        ".micro/extensions/prompt-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerTool({{
        name: "deploy_tool",
        description: "deploys the app",
        promptSnippet: "use this to deploy",
        promptGuidelines: ["always confirm first"],
        execute: async () => "deployed",
    }});
    micro.registerCommand("probe-prompt", {{
        handler: async (args, ctx) => {{
            writeFileSync({log:?}, JSON.stringify(ctx.getSystemPromptOptions()));
            return "probed";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "-t", "deploy_tool", "/probe-prompt"]);
    assert!(output.status.success(), "{}", output.stderr);
    assert!(output.stdout.contains("probed"), "{}", output.stdout);

    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&log).expect("the extension wrote a log"))
            .expect("the log is JSON");

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
            file["path"].as_str().unwrap_or_default().ends_with("AGENTS.md")
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

/// `ctx.scopedModels` reflects a `-c scoped_models=` setting, resolved to real catalog
/// entries rather than left as the raw pattern.
#[test]
fn scoped_models_reach_the_extension_context_resolved() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("scoped.log");
    fixture.write(
        ".micro/extensions/scope-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe-scope", {{
        handler: async (args, ctx) => {{
            writeFileSync({log:?}, JSON.stringify(ctx.scopedModels));
            return "scoped";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let scoped = fixture.print(&[
        "-m",
        "test",
        "-c",
        r#"scoped_models=["anthropic/claude-opus-5"]"#,
        "/probe-scope",
    ]);
    assert!(scoped.status.success(), "{}", scoped.stderr);

    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&log).expect("the extension wrote a log"))
            .expect("the log is JSON");
    let matches = read.as_array().expect("a list");
    assert!(
        matches
            .iter()
            .any(|entry| entry["model"]["provider"] == "anthropic"
                && entry["model"]["id"] == "claude-opus-5"),
        "{matches:?}"
    );
}

/// With nothing scoped, `ctx.scopedModels` is empty — pi's own reading of "unscoped",
/// rather than the whole catalog standing in for it.
#[test]
fn unscoped_models_reach_the_extension_context_as_empty() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("unscoped.log");
    fixture.write(
        ".micro/extensions/unscoped-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe-unscoped", {{
        handler: async (args, ctx) => {{
            writeFileSync({log:?}, JSON.stringify(ctx.scopedModels));
            return "unscoped";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe-unscoped"]);
    assert!(output.status.success(), "{}", output.stderr);
    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&log).expect("the extension wrote a log"))
            .expect("the log is JSON");
    assert_eq!(read, serde_json::json!([]));
}

/// A tool call gets pi's plain `ExtensionContext`, not the command-only one: the members
/// that move the conversation somewhere else are for a command handler to use, not a tool
/// the model calls mid-turn.
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
    let log = fixture.workspace().join("tool-context.log");
    fixture.write(
        ".micro/extensions/tool-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerTool({{
        name: "check",
        description: "reports what its context carries",
        execute: async (toolCallId, args, signal, onUpdate, ctx) => {{
            writeFileSync({log:?}, JSON.stringify({{
                hasModel: ctx.model !== undefined,
                hasNewSession: typeof ctx.newSession === "function",
                hasFork: typeof ctx.fork === "function",
                hasNavigateTree: typeof ctx.navigateTree === "function",
                hasSwitchSession: typeof ctx.switchSession === "function",
                hasReload: typeof ctx.reload === "function",
            }}));
            return "checked";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "run the check"]);
    assert!(output.status.success(), "{}", output.stderr);

    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&log).expect("the tool wrote a log"))
            .expect("the log is JSON");
    // The tool still gets the plain context — `model` and the rest of
    // `ExtensionContext` are there — but not the five that belong to a command.
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

/// An extension's tool can report partial progress while it runs, through `onUpdate` —
/// and that shows up as its own event, before the tool's final result, the same way a
/// streaming built-in like `bash` is watchable rather than silent until it is done.
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

    // The update arrived while the call was still running, not after the fact.
    let end_index = lines
        .iter()
        .position(|line| line["type"] == "tool_end" && line["name"] == "narrate")
        .expect("the call finished");
    assert!(
        update_index < end_index,
        "the update should precede the result: {lines:#?}"
    );
}

/// An extension's tool sees the turn's own abort: dropping the turn does not leave the
/// call running forever with nobody watching, and the extension can tell it happened.
#[test]
fn an_extension_tool_is_stopped_when_the_turn_is_aborted() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::tool_call("call_1", "wait_forever", json!({}))]);
    let fixture = Fixture::new(&api);
    let started = fixture.workspace().join("started.txt");
    let marker = fixture.workspace().join("aborted.txt");
    fixture.write(
        ".micro/extensions/waits.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerTool({{
        name: "wait_forever",
        description: "never resolves unless the turn is aborted",
        execute: async (toolCallId, args, signal) => {{
            writeFileSync({started:?}, "started");
            return new Promise((resolve, reject) => {{
                signal?.addEventListener("abort", () => {{
                    writeFileSync({marker:?}, "aborted");
                    reject(new Error("aborted"));
                }});
            }});
        }},
    }});
}};
"#,
            started = started.display().to_string(),
            marker = marker.display().to_string()
        ),
    );

    // Driven by hand rather than through `Fixture::rpc`, which writes every command before
    // anything is read back: the abort has to land once the tool call is actually running,
    // not race it — the `started` marker is what tells this when that moment has come.
    use std::io::Write as _;
    let mut command = fixture.micro();
    command.arg("--rpc");
    command.args(["-m", "test"]);
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command.spawn().expect("micro --rpc starts");

    writeln!(
        child.stdin.as_mut().expect("stdin is piped"),
        r#"{{"type":"prompt","message":"wait for it","id":"1"}}"#
    )
    .expect("the prompt is written");

    let running_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !started.exists() && std::time::Instant::now() < running_deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(started.exists(), "the tool call should have started running");

    writeln!(
        child.stdin.as_mut().expect("stdin is piped"),
        r#"{{"type":"abort","id":"2"}}"#
    )
    .expect("the abort is written");
    // Closing stdin is what ends `--rpc`; without it the process waits for more commands.
    drop(child.stdin.take());
    let _ = child.wait_with_output().expect("micro --rpc finishes");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        marker.exists(),
        "the extension's tool call was told to stop when the turn was aborted"
    );
}

/// A theme is a file on disk, the same one micro itself reads, so an extension can look one
/// up, switch to it, and color text with it — all without a live interface to ask, since
/// `getAllThemes`, `getTheme` and `setTheme` resolve locally rather than over the wire.
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

    let log = fixture.workspace().join("theme.log");
    fixture.write(
        ".micro/extensions/theme-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async (args, ctx) => {{
            const all = ctx.ui.getAllThemes();
            const looked_up = ctx.ui.getTheme("mine");
            const missing = ctx.ui.getTheme("nocturne");
            const before = ctx.ui.theme.name;
            const switched = ctx.ui.setTheme("mine");
            const after = ctx.ui.theme.name;
            const colored = ctx.ui.theme.fg("accent", "hi");
            writeFileSync({log:?}, JSON.stringify({{
                names: all.map((t) => t.name),
                lookedUpName: looked_up?.name,
                lookedUpAccent: looked_up?.fg("accent", "x"),
                missing,
                before,
                switched,
                after,
                colored,
            }}));
            return "done";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let written = std::fs::read_to_string(&log).expect("the extension wrote its findings");
    let result: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
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

/// `getEditorText` and `getToolsExpanded` answer synchronously, the same as they do in pi,
/// which is only possible here because each echoes back what this extension itself last
/// set rather than reading live state through a pipe. Both are exercised together since
/// neither needs a live interface to prove out.
#[test]
fn an_extension_reads_back_what_it_set_in_the_editor_and_in_tools_expansion() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("echo.log");
    fixture.write(
        ".micro/extensions/echo-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async (args, ctx) => {{
            const beforeText = ctx.ui.getEditorText();
            ctx.ui.setEditorText("hello");
            const afterSet = ctx.ui.getEditorText();
            ctx.ui.pasteToEditor(" world");
            const afterPaste = ctx.ui.getEditorText();

            const beforeExpanded = ctx.ui.getToolsExpanded();
            ctx.ui.setToolsExpanded(true);
            const afterExpanded = ctx.ui.getToolsExpanded();

            writeFileSync({log:?}, JSON.stringify({{
                beforeText, afterSet, afterPaste, beforeExpanded, afterExpanded,
            }}));
            return "done";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let written = std::fs::read_to_string(&log).expect("the extension wrote its findings");
    let result: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    assert_eq!(result["beforeText"], "");
    assert_eq!(result["afterSet"], "hello");
    assert_eq!(result["afterPaste"], "hello world");
    assert_eq!(result["beforeExpanded"], false);
    assert_eq!(result["afterExpanded"], true);
}

/// Every member of `ExtensionUIContext` that hands back a live TUI component — a header, a
/// footer, a widget, the editor, `custom`'s result — really does run the factory it is
/// given and register what it returns, rather than pretending to: the object stays in this
/// process and micro would drive it by id, the same way `execute()` already runs here and
/// is called from there. `--print` has no interface to open an overlay or hold a
/// replacement editor in, so `custom()` still resolves `undefined` and nothing drawn by any
/// of these actually reaches the (nonexistent) screen — but none of it throws, and
/// `getEditorComponent` hands back the very factory `setEditorComponent` was just given.
#[test]
fn an_extension_asking_for_a_live_component_runs_the_factory_it_is_given() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("components.log");
    fixture.write(
        ".micro/extensions/components-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
const component = () => ({{ render: (width) => [`hi ${{width}}`] }});
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async (args, ctx) => {{
            const customResult = await ctx.ui.custom(() => ({{
                render: () => ["overlay"],
                handleInput: () => {{}},
            }}));
            ctx.ui.setHeader(component);
            ctx.ui.setFooter(component);
            ctx.ui.addAutocompleteProvider((current) => current);
            const editorFactory = component;
            ctx.ui.setEditorComponent(editorFactory);
            const editorComponent = ctx.ui.getEditorComponent();
            ctx.ui.setWidget("factory-widget", component);

            writeFileSync({log:?}, JSON.stringify({{
                customResult: customResult === undefined,
                editorComponentIsTheSameFactory: editorComponent === editorFactory,
                ranWithoutThrowing: true,
            }}));
            return "done";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let written = std::fs::read_to_string(&log).expect("the extension wrote its findings");
    let result: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    assert_eq!(result["customResult"], true);
    assert_eq!(result["editorComponentIsTheSameFactory"], true);
    assert_eq!(result["ranWithoutThrowing"], true);
}

/// `setWidget`'s component-factory overload cannot be sent over the wire when it is a
/// function pi-tui itself cannot serialize either — the object stays in this process, but
/// the factory is still called and its component still registered, the same as every other
/// live-component member; only the JSON `send` this process makes of it is impossible, and
/// that only matters once micro is on the other end to ask for it, which `--print` is not.
#[test]
fn setting_a_widget_component_never_throws_even_headless() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("widget-component.log");
    fixture.write(
        ".micro/extensions/widget-component-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async (args, ctx) => {{
            ctx.ui.setWidget("status", (tui) => ({{
                render: (width) => [`status at ${{width}}`],
                invalidate: () => {{}},
            }}));
            writeFileSync({log:?}, "ok");
            return "done";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "ok");
}

/// `onTerminalInput` registers and unregisters without a live terminal to read keys from —
/// the registration itself is ordinary, synchronous bookkeeping; only actually being
/// offered a key needs the interactive interface `--print` does not have.
#[test]
fn an_extension_can_register_and_unregister_a_terminal_input_listener() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("listener.log");
    fixture.write(
        ".micro/extensions/listener-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async (args, ctx) => {{
            const unsubscribe = ctx.ui.onTerminalInput((data) => undefined);
            const isFunction = typeof unsubscribe === "function";
            unsubscribe();
            writeFileSync({log:?}, JSON.stringify({{ isFunction }}));
            return "done";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let written = std::fs::read_to_string(&log).expect("the extension wrote its findings");
    let result: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    assert_eq!(result["isFunction"], true);
}

/// A pi extension imports pi's own runtime modules — `@earendil-works/pi-coding-agent`,
/// `@earendil-works/pi-tui`, and their older `@mariozechner/*` names — the same way the
/// real `pi-subagents` extension does. Without something to resolve those to, the import
/// fails and the extension never loads at all. What micro's compatibility layer answers
/// for real (pi-tui's text measurement, pi-coding-agent's pure helpers) is exercised here
/// for a genuine, computed answer; what it does not answer for (pi's own agent loop and
/// interactive TUI) is exercised for the specific, named failure reaching for it produces
/// — at the point of use, not at import time, since the module loads regardless.
#[test]
fn an_extension_importing_a_pi_runtime_module_loads_and_runs() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("pi-runtime.log");
    fixture.write(
        ".micro/extensions/pi-runtime-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
import {{ stripTerminalSequences, truncateToWidth, visibleWidth }} from "@earendil-works/pi-tui";
import {{ CONFIG_DIR_NAME, defineTool }} from "@mariozechner/pi-coding-agent";
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async () => {{
            const width = visibleWidth("hello");
            // Real pi-tui truncateToWidth wraps its answer with a style reset even for
            // plain text, so what an extension actually reads is stripped through
            // pi-tui's own stripTerminalSequences before it is compared to anything.
            const truncated = stripTerminalSequences(truncateToWidth("hello world", 5));
            const tool = defineTool({{ name: "x" }});
            let unsupportedError;
            try {{
                const mod = await import("@earendil-works/pi-coding-agent");
                mod.main();
            }} catch (error) {{
                unsupportedError = error instanceof Error ? error.message : String(error);
            }}
            writeFileSync(
                {log:?},
                JSON.stringify({{
                    width,
                    truncated,
                    configDirName: CONFIG_DIR_NAME,
                    toolName: tool.name,
                    unsupportedError,
                }}),
            );
            return "done";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let written = std::fs::read_to_string(&log).expect("the extension wrote its findings");
    let result: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    assert_eq!(result["width"], 5, "visibleWidth answered for real");
    assert_eq!(result["truncated"], "he...", "truncateToWidth answered for real");
    assert_eq!(
        result["configDirName"], ".micro",
        "CONFIG_DIR_NAME answers with micro's own directory name, not pi's — an extension \
         building a path from it should land somewhere micro actually reads"
    );
    assert_eq!(result["toolName"], "x", "defineTool is pi's own identity function");
    let error = result["unsupportedError"]
        .as_str()
        .expect("main() is not supported here, and says so");
    assert!(
        error.contains("main"),
        "the failure names what was actually reached for: {error}"
    );
}

/// pi-tui's layout components (`HStack`/`VStack`), `TruncatedText`, `renderLatex` and the
/// autocomplete provider contract are all pure — no terminal I/O — and real in this
/// compatibility layer rather than stubbed. This exercises each for a genuinely computed
/// answer, not merely that importing them does not throw.
#[test]
fn an_extension_uses_pi_tuis_pure_layout_and_autocomplete_components() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("pi-tui-components.log");
    fixture.write(
        ".micro/extensions/pi-tui-components-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
import {{
    CombinedAutocompleteProvider,
    HStack,
    renderLatex,
    stripTerminalSequences,
    Text,
    TruncatedText,
    VStack,
}} from "@earendil-works/pi-tui";
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async () => {{
            // Two 3-wide texts side by side in a 10-wide box: real column layout, not a stub.
            const hstack = new HStack([new Text("aaa", 0, 0), new Text("bbb", 0, 0)], {{ gap: 1 }});
            // compositeTuiLine wraps each composited segment in a style-reset even for
            // plain text (see tui.ts), so the layout itself is read through
            // stripTerminalSequences the same way truncateToWidth's answer is above.
            const hstackLines = hstack.render(10).map((line) => stripTerminalSequences(line));

            // Stacked vertically instead: line count reflects both children plus the gap.
            const vstack = new VStack([new Text("one", 0, 0), new Text("two", 0, 0)]);
            const vstackLines = vstack.render(10);

            const truncated = stripTerminalSequences(
                new TruncatedText("a very long line that will not fit", 0, 0).render(10)[0],
            );

            const latex = renderLatex("x^2");

            const provider = new CombinedAutocompleteProvider(
                [{{ name: "help", description: "show help" }}],
                process.cwd(),
                null,
            );
            const suggestions = await provider.getSuggestions(["/hel"], 0, 4, {{ signal: new AbortController().signal }});

            writeFileSync(
                {log:?},
                JSON.stringify({{
                    hstackLineCount: hstackLines.length,
                    hstackFirstLine: hstackLines[0],
                    vstackLineCount: vstackLines.length,
                    truncated,
                    latex,
                    suggestionNames: suggestions?.items.map((item) => item.value) ?? null,
                }}),
            );
            return "done";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let written = std::fs::read_to_string(&log).expect("the extension wrote its findings");
    let result: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    assert_eq!(result["hstackLineCount"], 1, "one line of text laid out side by side stays one line");
    assert_eq!(
        result["hstackFirstLine"], "aaa bbb   ",
        "HStack actually composited both children with the gap between them, padded to the full requested width"
    );
    assert_eq!(result["vstackLineCount"], 2, "two stacked single-line children produce two lines");
    assert_eq!(result["truncated"], "a very ...", "TruncatedText actually truncated to the given width");
    assert_eq!(result["latex"], "x²", "renderLatex actually rendered the expression, not a stub answer");
    assert_eq!(
        result["suggestionNames"],
        serde_json::json!(["help"]),
        "CombinedAutocompleteProvider actually matched the slash command"
    );
}

/// An extension's command handler that throws is not a command that quietly did nothing —
/// it is a run that failed, the same way pi's own print mode exits nonzero for a command
/// that raised rather than answered. Before this, `--print` printed the error and still
/// exited zero, which is indistinguishable from success to anything scripting it.
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

/// pi-tui's `Markdown` component is real now that `marked` is vendored — genuine
/// markdown tokenization and rendering, not a stub. `theme.underline` is not exercised
/// here on purpose: coding-agent/index.ts's `getMarkdownTheme()` does not implement it
/// yet (flagged separately), and this test's job is `Markdown` itself, not that gap.
#[test]
fn an_extension_renders_real_markdown() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("markdown.log");
    fixture.write(
        ".micro/extensions/markdown-probe.ts",
        &format!(
            r##"
import {{ writeFileSync }} from "node:fs";
import {{ Markdown, stripTerminalSequences }} from "@earendil-works/pi-tui";
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async () => {{
            const theme = {{
                heading: (t) => t,
                link: (t) => t,
                linkUrl: (t) => t,
                code: (t) => `[${{t}}]`,
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
            }};
            const markdown = new Markdown("# Title\n\nSome `code` here.", 0, 0, theme);
            const lines = markdown.render(40).map((line) => stripTerminalSequences(line));
            writeFileSync({log:?}, JSON.stringify({{ lines }}));
            return "done";
        }},
    }});
}};
"##,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let written = std::fs::read_to_string(&log).expect("the extension wrote its findings");
    let result: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    let lines: Vec<String> = result["lines"]
        .as_array()
        .expect("lines is an array")
        .iter()
        .map(|line| line.as_str().unwrap_or_default().to_string())
        .collect();
    let joined = lines.join("\n");
    assert!(joined.contains("Title"), "the heading text made it through: {joined:?}");
    assert!(
        joined.contains("[code]"),
        "inline code was actually tokenized by marked and styled through the theme's code fn: {joined:?}"
    );
}

/// `custom()` and `setEditorComponent()` hand their factory pi-tui's real `KeybindingsManager`
/// now, not `{}` — an extension whose factory calls `keybindings.matches(data, name)` the way
/// pi's own `CustomEditor.handleInput` does would have thrown `TypeError: keybindings.matches
/// is not a function` on the very first bound keypress before this. `escape` is bound to
/// `tui.select.cancel` by pi-tui's own default table, so a real manager answers `true` for it
/// and `false` for a key that was never bound to it — not just "does not throw."
#[test]
fn a_components_keybindings_argument_answers_for_real_rather_than_being_empty() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    let log = fixture.workspace().join("keybindings-probe.log");
    fixture.write(
        ".micro/extensions/keybindings-probe.ts",
        &format!(
            r#"
import {{ writeFileSync }} from "node:fs";
export default (micro) => {{
    micro.registerCommand("probe", {{
        handler: async (args, ctx) => {{
            const seen = [];
            const factory = (tui, theme, keybindings) => {{
                seen.push(keybindings.matches("\x1b", "tui.select.cancel"));
                seen.push(keybindings.matches("x", "tui.select.cancel"));
                return {{ render: () => ["editor"], handleInput: () => {{}} }};
            }};
            ctx.ui.setEditorComponent(factory);
            // `--print` has no interface to open an overlay in, so `custom()` resolves
            // `undefined` headless regardless of `done` — see
            // `an_extension_asking_for_a_live_component_runs_the_factory_it_is_given`. What
            // this is checking is only that the factory's `keybindings` argument works.
            await ctx.ui.custom((tui, theme, keybindings, done) => {{
                seen.push(keybindings.matches("\x1b", "tui.select.cancel"));
                done("closed");
                return {{ render: () => ["overlay"], handleInput: () => {{}} }};
            }});
            writeFileSync({log:?}, JSON.stringify({{ seen }}));
            return "done";
        }},
    }});
}};
"#,
            log = log.display().to_string()
        ),
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    assert!(output.status.success(), "{}", output.stderr);

    let written = std::fs::read_to_string(&log).expect("the extension wrote its findings");
    let result: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
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
