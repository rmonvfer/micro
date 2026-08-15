//! Empirical parity check, not a fix: do pi's own example extensions load in micro?

mod support;

use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;
use support::FakeApi;
use support::Fixture;
use support::Reply;


const TIMEOUT: Duration = Duration::from_secs(30);

/// Where this repo keeps its own copy of pi's example extensions.
fn vendored_extensions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions")
}

/// One example extension: a single file, or a directory carrying its own entry point.
struct Example {
    name: &'static str,
    directory: bool,
    /// A caveat worth printing alongside the result.
    note: Option<&'static str>,
    /// Set for the few vendored examples that declare a real third-party npm dependency in their
    /// own `package.json`.
    needs_own_deps: bool,
}

const fn file(name: &'static str) -> Example {
    Example {
        name,
        directory: false,
        note: None,
        needs_own_deps: false,
    }
}

const fn file_with_note(name: &'static str, note: &'static str) -> Example {
    Example {
        name,
        directory: false,
        note: Some(note),
        needs_own_deps: false,
    }
}

const fn dir(name: &'static str) -> Example {
    Example {
        name,
        directory: true,
        note: None,
        needs_own_deps: false,
    }
}

const fn dir_with_note(name: &'static str, note: &'static str) -> Example {
    Example {
        name,
        directory: true,
        note: Some(note),
        needs_own_deps: false,
    }
}


const fn dir_needs_deps(name: &'static str, note: &'static str) -> Example {
    Example {
        name,
        directory: true,
        note: Some(note),
        needs_own_deps: true,
    }
}

/// Every example under `examples/extensions`, README-ordered.
const EXAMPLES: &[Example] = &[
    
    file("permission-gate"),
    file("project-trust"),
    file("protected-paths"),
    file("confirm-destructive"),
    file("dirty-repo-guard"),
    dir_needs_deps(
        "sandbox",
        "declares a real npm dependency (@anthropic-ai/sandbox-runtime)",
    ),
    dir_needs_deps(
        "gondolin",
        "declares a real npm dependency (@earendil-works/gondolin)",
    ),
    
    file("todo"),
    file("hello"),
    file_with_note("question", "exercises ctx.ui.select(), a live terminal"),
    file_with_note("questionnaire", "exercises a tab-bar UI, a live terminal"),
    file("tool-override"),
    file("dynamic-tools"),
    file("kimi-deferred-tools"),
    file("structured-output"),
    file("built-in-tool-renderer"),
    file("minimal-mode"),
    file("truncated-tool"),
    file("ssh"),
    dir("subagent"),
    
    file("preset"),
    dir("plan-mode"),
    file("tools"),
    file("handoff"),
    file_with_note("qna", "writes into the editor via ctx.ui.setEditorText(), a live terminal"),
    file_with_note("status-line", "renders into the footer, a live terminal"),
    file_with_note(
        "github-issue-autocomplete",
        "shells out to the real `gh` CLI against a real repo's issues",
    ),
    file_with_note("widget-placement", "renders widgets around the editor, a live terminal"),
    file_with_note("hidden-thinking-label", "customizes a collapsed-thinking label, a live terminal"),
    file_with_note("working-indicator", "customizes the streaming indicator, a live terminal"),
    file("model-status"),
    file_with_note("snake", "a keyboard-driven game, a live terminal"),
    file_with_note("tic-tac-toe", "a keyboard-driven game, a live terminal"),
    file("send-user-message"),
    file_with_note("timed-confirm", "exercises ctx.ui.confirm()/select(), a live terminal"),
    file_with_note("rpc-demo", "exercises RPC-supported extension UI methods interactively"),
    file_with_note("modal-editor", "replaces the editor component, a live terminal"),
    file_with_note("rainbow-editor", "an animated custom editor, a live terminal"),
    file("notify"),
    file_with_note("titlebar-spinner", "animates the terminal title, a live terminal"),
    file("summarize"),
    file_with_note("custom-footer", "renders a custom footer, a live terminal"),
    file_with_note("custom-header", "renders a custom header, a live terminal"),
    file_with_note("overlay-test", "overlay compositing tests, a live terminal"),
    file_with_note("overlay-qa-tests", "overlay compositing tests, a live terminal"),
    dir_with_note("doom-overlay", "a real-time game rendered as an overlay, a live terminal"),
    file("shutdown-command"),
    file("reload-runtime"),
    file("commands"),
    file_with_note("interactive-shell", "runs vim/htop with a full terminal via user_bash"),
    file("inline-bash"),
    file("input-transform-streaming"),
    
    file("input-transform"),
    file("bash-spawn-hook"),
    file("border-status-editor"),
    file_with_note("entry-renderer", "TUI-only session entry rendering"),
    file("event-bus"),
    file("git-merge-and-resolve"),
    file_with_note("message-renderer", "custom message rendering, a live terminal"),
    file("prompt-customizer"),
    file("provider-payload"),
    file_with_note("space-invaders", "a keyboard-driven game, a live terminal"),
    file("system-prompt-header"),
    file("working-message-test"),
    file("mac-system-theme"),
    
    file("git-checkpoint"),
    file("auto-commit-on-exit"),
    
    file("pirate"),
    file("claude-rules"),
    file("custom-compaction"),
    file("trigger-compact"),
    
    dir("dynamic-resources"),
    
    file("session-name"),
    file("bookmark"),
    
    dir_needs_deps(
        "custom-provider-anthropic",
        "declares a real npm dependency (@anthropic-ai/sdk)",
    ),
    dir_with_note(
        "custom-provider-gitlab-duo",
        "registers a real provider; not exercised end to end here, only loaded",
    ),
    
    dir_needs_deps("with-deps", "declares a real npm dependency (ms)"),
    file_with_note("file-trigger", "watches a file for changes across the run"),
];

/// Recursively copy a directory-based extension exactly as vendored.
fn copy_dir_all(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap_or_else(|error| panic!("create {}: {error}", dst.display()));
    let entries = std::fs::read_dir(src).unwrap_or_else(|error| panic!("read {}: {error}", src.display()));
    for entry in entries {
        let entry = entry.expect("a directory entry reads");
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_all(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap_or_else(|error| panic!("copy {}: {error}", path.display()));
        }
    }
}

/// What happened when micro was started with exactly this one extension present.
struct Attempt {
    /// The extension host's own "was not loaded" note, verbatim, when it reported one.
    load_failure: Option<String>,
    /// Whether the process ran to completion within [`TIMEOUT`], successfully or not.
    finished: bool,
    exit_ok: bool,
    stderr: String,
}


fn run_with_timeout(mut command: std::process::Command) -> Attempt {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn the micro binary");
    let mut stdout_pipe = child.stdout.take().expect("stdout is piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr is piped");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = String::new();
        let _ = stdout_pipe.read_to_string(&mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = String::new();
        let _ = stderr_pipe.read_to_string(&mut buffer);
        buffer
    });

    let deadline = Instant::now() + TIMEOUT;
    let (finished, exit_ok) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (true, status.success()),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break (false, false);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break (false, false),
        }
    };

    
    let _stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();

    Attempt {
        load_failure: stderr
            .lines()
            .find(|line| line.contains("was not loaded:"))
            .map(str::to_string),
        finished,
        exit_ok,
        stderr,
    }
}

/// Install one example into a fresh fixture and run one plain-text turn through it.
fn attempt(example: &Example) -> Attempt {
    let api = FakeApi::start([Reply::text("ok")]);
    let fixture = Fixture::new(&api);
    let mut command = fixture.micro();

    if example.needs_own_deps {
        let source_path = vendored_extensions_dir().join(example.name);
        let source = micro_extensions::Source::parse(&source_path.display().to_string())
            .unwrap_or_else(|error| panic!("{}: {error}", source_path.display()));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build a runtime for the install call");
        match runtime.block_on(micro_extensions::install(
            &source,
            &fixture.home(),
            &fixture.workspace(),
            true,
        )) {
            Ok(installed) => {
                command.arg("--extension").arg(installed.path);
            }
            Err(reason) => {
                return Attempt {
                    load_failure: Some(format!("micro's own install path refused it: {reason}")),
                    finished: true,
                    exit_ok: false,
                    stderr: String::new(),
                };
            }
        }
    } else if example.directory {
        let source = vendored_extensions_dir().join(example.name);
        let dest = fixture
            .workspace()
            .join(".micro/extensions")
            .join(example.name);
        copy_dir_all(&source, &dest);
    } else {
        let source = vendored_extensions_dir().join(format!("{}.ts", example.name));
        let content = std::fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("read {}: {error}", source.display()));
        fixture.write(&format!(".micro/extensions/{}.ts", example.name), &content);
    }

    command.args(["--print", "-m", "test", "say hi"]);
    run_with_timeout(command)
}

/// Loads every vendored example extension and reports, per extension, whether it loaded.
#[test]
fn example_extensions_load_report() {
    if micro_extensions::which_bun().is_none() {
        eprintln!("skipped: bun is not on the path, so nothing here could load anyway");
        return;
    }
    assert!(
        vendored_extensions_dir().is_dir(),
        "expected the vendored example extensions at {}",
        vendored_extensions_dir().display()
    );

    let mut lines = vec![
        "| extension | kind | install path | result | caveat |".to_string(),
        "|---|---|---|---|---|".to_string(),
    ];
    let mut loaded = 0usize;
    let mut failed = 0usize;

    for example in EXAMPLES {
        let attempt = attempt(example);
        let kind = if example.directory { "dir" } else { "file" };
        let path = if example.needs_own_deps {
            "installed (micro_extensions::install)"
        } else {
            "copied into .micro/extensions/"
        };
        let caveat = example.note.unwrap_or("");
        let result = match (&attempt.load_failure, attempt.finished, attempt.exit_ok) {
            (Some(note), _, _) => {
                failed += 1;
                let reason = note.trim_start_matches("note: ");
                format!("FAIL — {reason}")
            }
            (None, false, _) => {
                failed += 1;
                "FAIL — timed out; likely blocked on a live terminal this fixture has none of".to_string()
            }
            (None, true, false) => {
                failed += 1;
                let tail: String = attempt.stderr.lines().rev().take(3).collect::<Vec<_>>().join(" / ");
                format!("FAIL — loaded, but the run did not exit cleanly ({tail})")
            }
            (None, true, true) => {
                loaded += 1;
                "OK — loaded, one plain turn ran to completion".to_string()
            }
        };
        lines.push(format!("| {} | {kind} | {path} | {result} | {caveat} |", example.name));
    }

    lines.push(String::new());
    lines.push(format!(
        "{loaded} of {} loaded and ran a plain turn cleanly; {failed} did not.",
        EXAMPLES.len(),
    ));

    eprintln!("\n{}", lines.join("\n"));
}
