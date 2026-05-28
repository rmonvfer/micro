//! What an extension is allowed to ask micro for, proved through the real binary.

mod support;

use micro_extensions::which_bun;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;
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

/// Every crossing the session recorded, in the order they happened.
fn crossings(fixture: &Fixture, id: &str) -> Vec<Value> {
    let exported = Output::run(fixture.micro().args(["sessions", "export", id]));
    exported.expect_success("micro sessions export");
    exported
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line["event"]["type"] == "extension_crossing")
        .map(|line| line["event"].clone())
        .collect()
}


#[test]
fn an_ask_outside_the_manifest_is_refused_by_name_and_the_session_continues() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/narrow.ts",
        r#"
export const capabilities = ["commands"];

export default (micro) => {
    micro.registerCommand("probe", {
        handler: async () => {
            const result = await micro.exec("echo", ["hi"]);
            return `refused: ${result.error}`;
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    output.expect_success("micro --print /probe");
    assert!(
        output.stdout.contains("capability 'exec' not granted to narrow"),
        "the extension was not told why: {}",
        output.stdout
    );

    let recorded = crossings(&fixture, &only_session(&fixture));
    let refusal = recorded
        .iter()
        .find(|crossing| crossing["kind"] == "exec")
        .unwrap_or_else(|| panic!("no exec crossing among {recorded:?}"));
    assert_eq!(refusal["extension"], "narrow");
    assert_eq!(refusal["allowed"], false);
    assert_eq!(refusal["name"], "echo", "the ledger says what was asked for");
}


#[test]
fn an_ask_inside_the_manifest_is_answered_and_recorded() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/allowed.ts",
        r#"
export const capabilities = ["commands", "exec"];

export default (micro) => {
    micro.registerCommand("probe", {
        handler: async () => {
            const result = await micro.exec("echo", ["from an extension"]);
            return `said: ${(result.stdout ?? "").trim()}`;
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    output.expect_success("micro --print /probe");
    assert!(
        output.stdout.contains("said: from an extension"),
        "{}",
        output.stdout
    );

    let recorded = crossings(&fixture, &only_session(&fixture));
    let allowed = recorded
        .iter()
        .find(|crossing| crossing["kind"] == "exec")
        .unwrap_or_else(|| panic!("no exec crossing among {recorded:?}"));
    assert_eq!(allowed["extension"], "allowed");
    assert_eq!(allowed["allowed"], true);
}

/// Registering is an ask like any other.
#[test]
fn a_tool_registered_without_the_capability_is_never_offered_to_the_model() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("nothing to do")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/toolless.ts",
        r#"
export const capabilities = ["commands"];

export default (micro) => {
    micro.registerTool({
        name: "smuggled",
        description: "a tool nobody asked for",
        parameters: { type: "object", properties: {} },
        execute: async () => ({ content: [{ type: "text", text: "ran" }] }),
    });
    micro.registerCommand("probe", { handler: async () => "here" });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "say hi"]);
    output.expect_success("micro --print");
    assert!(
        output.stderr.contains("smuggled"),
        "the refusal names the tool: {}",
        output.stderr
    );

    let offered = support::offered_tools(&api.request(0));
    assert!(
        !offered.iter().any(|name| name == "smuggled"),
        "a tool that was refused was still offered: {offered:?}"
    );
}


#[test]
fn a_legacy_extension_in_a_trusted_project_keeps_working_unprompted() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/legacy.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async () => {
            const result = await micro.exec("echo", ["still working"]);
            return `said: ${(result.stdout ?? "").trim()}`;
        },
    });
};
"#,
    );

    let output = fixture.print(&["-m", "test", "/probe"]);
    output.expect_success("micro --print /probe");
    assert!(
        output.stdout.contains("said: still working"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stderr.contains("capabilities"),
        "nothing should have been asked or explained: {}",
        output.stderr
    );
}


#[test]
fn a_legacy_extension_in_an_untrusted_project_is_granted_nothing_and_says_why() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("nothing to do")]);
    let fixture = Fixture::new(&api);
    
    let path = fixture.write(
        "probe.ts",
        r#"
export default (micro) => {
    micro.registerCommand("probe", { handler: async () => "here" });
};
"#,
    );

    let output = fixture.print(&[
        "-m",
        "test",
        "--no-approve",
        "--extension",
        &path.display().to_string(),
        "say hi",
    ]);
    output.expect_success("micro --print");
    assert!(
        output.stderr.contains("declares no capabilities"),
        "the reason is said out loud: {}",
        output.stderr
    );
    assert!(
        output.stderr.contains("without asking for the `commands` capability"),
        "what was refused is named: {}",
        output.stderr
    );
    assert!(
        !output.stderr.contains("was not loaded"),
        "being granted nothing is not a failure to load: {}",
        output.stderr
    );
}


const EXIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Run to completion or [`EXIT_TIMEOUT`], and say whether the process ever left.
fn exits_on_its_own(fixture: &Fixture, arguments: &[&str]) -> bool {
    let mut command = fixture.micro();
    command.args(arguments);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());

    let mut child = command.spawn().expect("spawn the micro binary");
    let deadline = Instant::now() + EXIT_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return false,
        }
    }
}

/// A finished run leaves.
#[test]
fn a_finished_run_exits_whether_or_not_extensions_were_loaded() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([Reply::text("done"), Reply::text("done")]);
    let fixture = Fixture::new(&api);
    fixture.write(
        ".micro/extensions/watcher.ts",
        r#"
export default (micro) => {
    micro.on("agent_end", () => {});
};
"#,
    );

    assert!(
        exits_on_its_own(&fixture, &["--print", "-m", "test", "say hi"]),
        "a run with an extension loaded never exited"
    );
    assert!(
        exits_on_its_own(
            &fixture,
            &["--print", "-m", "test", "--no-extensions", "say hi"]
        ),
        "a run with no extensions never exited"
    );
}


#[test]
fn listing_installed_packages_says_what_each_may_do() {
    if which_bun().is_none() {
        return;
    }
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    fixture.write(
        "package/package.json",
        r#"{ "name": "declared-package", "micro": { "extensions": ["index.ts"], "capabilities": ["commands", "exec"] } }"#,
    );
    fixture.write(
        "package/index.ts",
        r#"export default (micro) => { micro.registerCommand("probe", { handler: async () => "here" }); };"#,
    );

    let installed = fixture.micro_run(&[
        "install",
        &fixture.workspace().join("package").display().to_string(),
    ]);
    installed.expect_success("micro install");

    let listed = fixture.micro_run(&["list"]);
    listed.expect_success("micro list");
    assert!(
        listed.stdout.contains("commands, exec"),
        "the declared set is not listed: {}",
        listed.stdout
    );
    assert!(
        !listed.stdout.contains("legacy:"),
        "a declared set is not a derived one: {}",
        listed.stdout
    );
}
