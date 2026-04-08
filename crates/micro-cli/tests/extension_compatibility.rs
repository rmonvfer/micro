//! Empirical parity check, not a fix: do pi's own example extensions load in micro?
//!
//! Every extension exercised here is vendored, verbatim, from pi's example extensions —
//! real code written against pi's API, not a fixture written to pass — into
//! `examples/extensions/` at the repo root, which this harness treats as its own. What
//! moved: only the files this repo controls. What did not move: the module specifiers
//! inside them (`@earendil-works/pi-ai`, `@earendil-works/pi-coding-agent`, and the rest)
//! are the compatibility contract third-party extension source actually imports, and stay
//! exactly as pi wrote them.
//!
//! The one thing checked for every extension, headlessly, is whether it loads: whether the
//! extension host runs its top-level registration without reporting the file as failed.
//! That is `runtime.rs`'s own signal — `note: {path} was not loaded: {error}` on stderr —
//! so this harness reads exactly what a person running micro would see, not something
//! reconstructed from internals.
//!
//! An extension whose whole point is a live terminal — a game, a custom editor, an overlay
//! — still gets the load check, because loading needs no terminal. Its *interactive*
//! behavior is a different question this harness cannot answer without one, and is called
//! out per-extension in the report rather than silently assumed to work.
//!
//! Neither micro nor this harness ever runs a package manager directly. micro deliberately
//! never auto-installs a dropped-in extension's dependencies at load time — that would
//! reopen the hole `--no-install` closed, where a file appearing in an extensions folder
//! makes micro fetch and run arbitrary code unprompted. Installing is its own explicit step,
//! the same as pi's own docs ask for, and micro's supported form of it is
//! `micro_extensions::install` — what `micro extension install` runs, and what an extension
//! declaring its own `package.json` goes through here instead of being copied straight into
//! `.micro/extensions/`. The report says which path each extension took.
//!
//! A git-sourced install fetches the package's own dependencies after cloning it — see
//! `micro-extensions/src/packages.rs`. A local-path install does not, and neither does pi's:
//! `installParsedSource` in pi's own `package-manager.ts` only installs for its `"npm"` and
//! `"git"` source types, nothing runs for `"local"`. The four vendored examples that declare
//! a real dependency are local directories, so they are expected to still report a missing
//! module even once installed through `micro_extensions::install` — that is not a gap next
//! to pi, it is the same manual-setup posture pi itself takes for a bare local folder.
//!
//! This file and its fixtures are the only things owned here. It does not, and must not,
//! edit `crates/micro-extensions/host/**`, `crates/micro-extensions/src/host.rs`, or
//! `host/sdk/*.ts` — those belong to the agents building the SDK surface this measures.

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

/// How long one extension gets to load, run a turn, and exit before it is judged hung
/// rather than merely slow. Generous: a real Bun process starts and imports real code.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Where this repo keeps its own copy of pi's example extensions. Resolved from the crate
/// manifest directory so the check does not depend on the working directory a test runner
/// happens to use.
fn vendored_extensions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions")
}

/// One example extension: a single file, or a directory carrying its own entry point.
struct Example {
    name: &'static str,
    directory: bool,
    /// A caveat worth printing alongside the result — most often that the extension's own
    /// point is interactive and a load pass says nothing about whether it actually works.
    note: Option<&'static str>,
    /// Set for the few vendored examples that declare a real third-party npm dependency in
    /// their own `package.json`. These are installed through `micro_extensions::install`
    /// and named with `--extension` instead of being copied into `.micro/extensions/` —
    /// see `attempt` — and the report says which path each extension took.
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

/// A directory-based example that declares a real third-party npm dependency: `note`
/// documents which one, and `needs_own_deps` routes it through micro's own install path
/// rather than a plain copy.
const fn dir_needs_deps(name: &'static str, note: &'static str) -> Example {
    Example {
        name,
        directory: true,
        note: Some(note),
        needs_own_deps: true,
    }
}

/// Every example under `examples/extensions`, README-ordered. `note` marks the ones this
/// harness can load-check but not exercise for real: a live terminal, a real git repo, a
/// network call to a real service, or another process this scratch fixture does not
/// provide.
const EXAMPLES: &[Example] = &[
    // Lifecycle & Safety
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
    // Custom Tools
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
    // Commands & UI
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
    // Present on disk, not listed in the README's table
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
    // Git Integration
    file("git-checkpoint"),
    file("auto-commit-on-exit"),
    // System Prompt & Compaction
    file("pirate"),
    file("claude-rules"),
    file("custom-compaction"),
    file("trigger-compact"),
    // Resources
    dir("dynamic-resources"),
    // Session Metadata
    file("session-name"),
    file("bookmark"),
    // Custom Providers
    dir_needs_deps(
        "custom-provider-anthropic",
        "declares a real npm dependency (@anthropic-ai/sdk)",
    ),
    dir_with_note(
        "custom-provider-gitlab-duo",
        "registers a real provider; not exercised end to end here, only loaded",
    ),
    // External Dependencies
    dir_needs_deps("with-deps", "declares a real npm dependency (ms)"),
    file_with_note("file-trigger", "watches a file for changes across the run"),
];

/// Recursively copy a directory-based extension exactly as vendored — nested modules,
/// `package.json`, everything — so discovery sees the same tree the example itself is.
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

/// Run `command` to completion or [`TIMEOUT`], whichever comes first, killing it rather
/// than leaving a hung child behind. Both pipes are drained on their own threads: a real
/// process's output can exceed the OS pipe buffer, and nothing would be reading it while
/// this thread only polls for exit.
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

    // Reading blocks until the pipe's write end closes, which killing the child ensures
    // even on the timeout path.
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

/// Install one example into a fresh fixture and run one plain-text turn through it — enough
/// to exercise registration, `session_start`, and whatever a normal turn's lifecycle events
/// touch, without depending on any tool the extension happens to declare.
///
/// A plain file or directory is copied straight into `.micro/extensions/`, the same as a
/// person dropping one in. An example that declares its own `package.json` instead goes
/// through micro's own supported install path (`micro_extensions::install`) and is then
/// named with `--extension`, the way a real package is added — neither micro nor this
/// harness ever runs a package manager directly.
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

/// Loads every vendored example extension and reports, per extension, whether it loaded —
/// the empirical answer to "does this extension work in micro unmodified," measured rather
/// than assumed.
///
/// Always passes: an extension not loading yet is the finding this harness exists to
/// surface, not a defect in the harness. Run with `--nocapture` (or `--test-threads=1
/// --nocapture` for a stable read while other tests are also printing) to see the table.
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
