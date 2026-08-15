//! What a session may touch, proved against the real program.
//!
//! The policy is enforced in two places for two different reasons — the kernel confines a
//! command, and the file tools check themselves because nothing else is in a position to —
//! so both are exercised here through a scripted turn rather than in isolation. What is
//! asserted is what the model is told and what the session recorded, which is the whole of
//! what a refusal is for.

mod support;

use micro_extensions::which_bun;
use serde_json::json;
use support::tool_results;
use support::FakeApi;
use support::Fixture;
use support::Reply;

/// Whether this platform actually confines a command.
///
/// Off the two platforms micro has a sandbox for, a command runs unconfined and says so, so
/// the cases about being refused have nothing to be about and are skipped rather than
/// asserting something that is not true there. Everything else — how a policy is resolved,
/// what the file tools do about it — holds everywhere and is checked everywhere.
fn enforced() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux"))
}

/// The whole of every session log this run wrote.
fn ledger(fixture: &Fixture) -> String {
    fixture.session_logs().join("\n")
}

/// A command that writes outside the workspace is stopped by the kernel, and the model is
/// told which policy stopped it rather than being left to read `exit code 1` and try again.
#[test]
fn a_command_writing_outside_the_workspace_is_refused_and_the_refusal_is_recorded() {
    if !enforced() {
        return;
    }

    let api = FakeApi::start([
        Reply::tool_call(
            "call_1",
            "bash",
            json!({ "command": "echo taken > ../outside.txt" }),
        ),
        Reply::text("I cannot write there."),
    ]);
    let fixture = Fixture::new(&api);
    let outside = fixture
        .workspace()
        .parent()
        .expect("a parent")
        .join("outside.txt");

    fixture
        .print(&["-m", "test", "write outside the workspace"])
        .expect_success("micro --print");

    let results = tool_results(&api.request(1));
    let said = results
        .first()
        .expect("the model was told what happened")
        .to_string();
    assert!(
        said.contains("denied by policy workspace-write"),
        "the model is told which policy refused: {said}"
    );
    assert!(
        !outside.exists(),
        "nothing was written outside the workspace"
    );

    let recorded = ledger(&fixture);
    assert!(
        recorded.contains("\"type\":\"sandbox_decision\""),
        "the refusal is a fact about the run: {recorded}"
    );
    assert!(
        recorded.contains("\"policy\":\"workspace-write\"")
            && recorded.contains("\"allowed\":false"),
        "under which policy, and which way it went: {recorded}"
    );
}

/// The same policy, the other enforcer. A file tool opens paths in micro's own process, so
/// no kernel is in a position to stop it: it asks first. A symlink is the way a path gets
/// past the lexical check that a path is inside the workspace and still lands outside it.
#[test]
fn a_file_tool_refuses_a_write_that_leaves_the_workspace_by_symlink() {
    let api = FakeApi::start([
        Reply::tool_call(
            "call_1",
            "write",
            json!({ "path": "escape/loot.txt", "content": "taken" }),
        ),
        Reply::text("I cannot write there."),
    ]);
    let fixture = Fixture::new(&api);
    let outside = fixture
        .workspace()
        .parent()
        .expect("a parent")
        .join("elsewhere");
    std::fs::create_dir_all(&outside).expect("somewhere outside to point at");
    std::os::unix::fs::symlink(&outside, fixture.workspace().join("escape"))
        .expect("a way out of the workspace");

    fixture
        .print(&["-m", "test", "write through the link"])
        .expect_success("micro --print");

    let said = tool_results(&api.request(1))
        .first()
        .expect("the model was told what happened")
        .to_string();
    assert!(
        said.contains("workspace-write"),
        "the model is told which policy refused: {said}"
    );
    assert!(!outside.join("loot.txt").exists(), "nothing was written");
    assert!(
        ledger(&fixture).contains("\"operation\":\"write\""),
        "the refusal says what was being attempted"
    );
}

/// Inside the workspace nothing changes: the policy is what a session works under, not
/// something it has to work around.
#[test]
fn the_workspace_itself_is_still_writable_under_the_default_policy() {
    let api = FakeApi::start([
        Reply::tool_call(
            "call_1",
            "bash",
            json!({ "command": "echo kept > notes.txt" }),
        ),
        Reply::text("written"),
    ]);
    let fixture = Fixture::new(&api);

    fixture
        .print(&["-m", "test", "write a note"])
        .expect_success("micro --print");

    assert_eq!(
        std::fs::read_to_string(fixture.workspace().join("notes.txt")).unwrap_or_default(),
        "kept\n"
    );
    assert!(
        !ledger(&fixture).contains("sandbox_decision"),
        "an ordinary turn leaves no sandbox decisions behind"
    );
}

/// Running unconfined is never quiet: it is said on the way in and written into the log,
/// because a session that could reach anything is the one fact a reader most needs.
#[test]
fn a_full_policy_says_so_and_is_recorded() {
    let api = FakeApi::start([Reply::text("hello")]);
    let fixture = Fixture::new(&api);

    let ran = fixture.print(&["-m", "test", "--sandbox", "full", "hello"]);
    ran.expect_success("micro --print --sandbox full");

    assert!(
        ran.stderr.contains("the sandbox is off"),
        "it is said out loud: {}",
        ran.stderr
    );
    let recorded = ledger(&fixture);
    assert!(
        recorded.contains("\"policy\":\"full\"") && recorded.contains("\"session-start\""),
        "and written down: {recorded}"
    );
}

/// A policy nobody recognizes ends the run. Falling back to a default would run the
/// session under something other than what was asked for, which is the one outcome a
/// policy exists to prevent.
#[test]
fn an_unknown_policy_stops_the_run_rather_than_choosing_one() {
    let api = FakeApi::start([Reply::text("hello")]);
    let fixture = Fixture::new(&api);

    let ran = fixture.print(&["-m", "test", "--sandbox", "yolo", "hello"]);
    ran.expect_failure("micro --print --sandbox yolo");
    assert!(ran.stderr.contains("yolo"), "{}", ran.stderr);
    assert!(
        ran.stderr.contains("workspace-write"),
        "the names it would have taken: {}",
        ran.stderr
    );
    assert_eq!(api.request_count(), 0, "nothing ran");
}

/// A trusted project chooses the policy its sessions run under, and the command line still
/// beats it.
#[test]
fn a_trusted_project_settles_the_policy_and_the_command_line_overrules_it() {
    let api = FakeApi::start([Reply::text("one"), Reply::text("two")]);
    let fixture = Fixture::new(&api);
    fixture.write(".micro/settings.json", r#"{"sandbox":"full"}"#);

    let ran = fixture.print(&["-m", "test", "hello"]);
    ran.expect_success("micro --print");
    assert!(
        ran.stderr.contains("the sandbox is off"),
        "the project asked for full: {}",
        ran.stderr
    );

    let overruled = fixture.print(&["-m", "test", "--sandbox", "workspace-write", "hello"]);
    overruled.expect_success("micro --print --sandbox workspace-write");
    assert!(
        !overruled.stderr.contains("the sandbox is off"),
        "the command line settles it: {}",
        overruled.stderr
    );
}

/// An extension runs under the session's policy too. It is code the user installed rather
/// than code the model wrote, but it reaches the same machine through the same host, and a
/// policy a program could step around by asking an extension to ask for it would not be a
/// policy.
#[test]
fn an_extension_running_a_command_is_held_to_the_same_policy() {
    if !enforced() || which_bun().is_none() {
        return;
    }

    let api = FakeApi::start([
        Reply::tool_call("call_1", "write_outside", json!({})),
        Reply::text("it would not let me"),
    ]);
    let fixture = Fixture::new(&api);
    let outside = fixture
        .workspace()
        .parent()
        .expect("a parent")
        .join("from-an-extension.txt");
    fixture.write(
        ".micro/extensions/escapee.ts",
        &format!(
            r#"
export default (micro) => {{
    micro.registerTool({{
        name: "write_outside",
        description: "Try to write outside the workspace",
        parameters: {{ type: "object", properties: {{}} }},
        execute: async () => {{
            const result = await micro.exec("/usr/bin/touch", ["{}"]);
            return JSON.stringify(result);
        }},
    }});
}};
"#,
            outside.display()
        ),
    );

    fixture
        .print(&["-m", "test", "run the extension's tool"])
        .expect_success("micro --print");

    // The extension hands back whatever `exec` answered, so what the model was told is
    // also what the extension saw: the command was refused, and by which policy.
    let said = tool_results(&api.request(1))
        .first()
        .expect("the extension's tool answered")
        .to_string();
    assert!(
        said.contains("denied") && said.contains("workspace-write"),
        "the extension was told its command was refused: {said}"
    );
    assert!(
        !outside.exists(),
        "an extension wrote outside the workspace: {}",
        outside.display()
    );
}

/// An untrusted project has no say at all. Widening what a session may do is exactly what
/// trust is asked about, so a checkout nobody has vouched for does not get to ask.
#[test]
fn an_untrusted_projects_policy_is_ignored() {
    let api = FakeApi::start([Reply::text("hello")]);
    let fixture = Fixture::new(&api);
    fixture.write(".micro/settings.json", r#"{"sandbox":"full"}"#);

    let ran = fixture.print(&["-m", "test", "--no-approve", "hello"]);
    ran.expect_success("micro --print --no-approve");
    assert!(
        !ran.stderr.contains("the sandbox is off"),
        "an untrusted project asked for full and did not get it: {}",
        ran.stderr
    );
}

/// What a user settled once applies to a run that says nothing, which is the whole point of
/// settling it.
#[test]
fn the_settled_policy_applies_when_nothing_else_says_otherwise() {
    if !enforced() {
        return;
    }

    let api = FakeApi::start([
        Reply::tool_call(
            "call_1",
            "bash",
            json!({ "command": "echo nope > notes.txt" }),
        ),
        Reply::text("I cannot write that."),
    ]);
    let fixture = Fixture::new(&api);
    std::fs::write(
        fixture.home().join("config.json"),
        r#"{"default_project_trust":"always","sandbox":"read-only"}"#,
    )
    .expect("settle a policy");

    fixture
        .print(&["-m", "test", "write a note"])
        .expect_success("micro --print");

    let said = tool_results(&api.request(1))
        .first()
        .expect("the model was told what happened")
        .to_string();
    assert!(said.contains("denied by policy read-only"), "{said}");
    assert!(!fixture.exists("notes.txt"), "nothing was written");
}

/// `micro sandbox try` answers the question the flags raise: what would this command
/// actually run as, and would it get through.
#[test]
fn sandbox_try_reports_what_it_ran_and_how_it_went() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let tried = fixture.micro_run(&["sandbox", "try", "--", "echo", "hello"]);
    tried.expect_success("micro sandbox try");
    assert!(
        tried.stdout.contains("policy: workspace-write"),
        "{}",
        tried.stdout
    );
    assert!(tried.stdout.contains("running:"), "{}", tried.stdout);
    assert!(tried.stdout.contains("exit:"), "{}", tried.stdout);
    assert!(
        tried.stdout.contains("hello"),
        "the command really ran: {}",
        tried.stdout
    );

    let named = fixture.micro_run(&[
        "sandbox",
        "try",
        "--sandbox",
        "read-only",
        "--",
        "echo",
        "hi",
    ]);
    named.expect_success("micro sandbox try --sandbox read-only");
    assert!(
        named.stdout.contains("policy: read-only"),
        "{}",
        named.stdout
    );

    if enforced() {
        let refused = fixture.micro_run(&["sandbox", "try", "--", "touch", "../outside.txt"]);
        assert!(
            refused.stdout.contains("looks denied: true"),
            "a command the policy stops is reported as one: {}",
            refused.stdout
        );
        assert!(
            !fixture
                .workspace()
                .parent()
                .expect("a parent")
                .join("outside.txt")
                .exists(),
            "and it really did not run"
        );
    }
}

/// And it says honestly whether anything is actually enforcing the policy, since a debug
/// command that reported confinement it does not have would be worse than none.
#[test]
fn sandbox_try_says_whether_the_policy_is_enforced_at_all() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let confined = fixture.micro_run(&["sandbox", "try", "--", "echo", "hi"]);
    confined.expect_success("micro sandbox try");
    let says_yes = confined.stdout.contains("enforced: yes");
    assert_eq!(
        says_yes,
        enforced(),
        "what it claims does not match the platform: {}",
        confined.stdout
    );

    let unconfined =
        fixture.micro_run(&["sandbox", "try", "--sandbox", "full", "--", "echo", "hi"]);
    unconfined.expect_success("micro sandbox try --sandbox full");
    assert!(
        unconfined.stdout.contains("enforced: no"),
        "`full` confines nothing, whatever the platform: {}",
        unconfined.stdout
    );
}
