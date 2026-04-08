//! Verifies that the terminal-only vendored extensions actually render and respond — not
//! only that they load. `extension_compatibility.rs` establishes the load half of that
//! question headlessly, for all of `examples/extensions/`; this file establishes the
//! behavior half, for the subset whose whole point is a terminal a headless run has none
//! of. Loading and working are different claims, and a caveat that only checked the first
//! one is not evidence about the second.
//!
//! The mechanism is a real pty: `tests/pty/drive.py` forks one, sets `TERM=xterm-256color`,
//! sizes the window, answers the two queries a real terminal must answer or the program
//! hangs waiting for them (a cursor-position report and a background-color query), sends
//! keystroke batches on a clock, and hands back the raw bytes a terminal would have shown.
//! It is Python because a working, hand-verified implementation of exactly this already
//! existed; re-deriving a correct raw pty driver in Rust from scratch would have meant
//! re-establishing correctness this session had already paid for once.
//!
//! Every check here looks for a specific, literal string that only appears on screen if
//! the extension's own interactive code path actually ran — the text `custom-header.ts`
//! prints in its subtitle, the label `working-indicator.ts` echoes back after a command,
//! the mode indicator `modal-editor.ts` draws in its default state. A pass means that
//! string reached the terminal; nothing here reconstructs it from source or internals.
//!
//! Not every terminal-only extension is checked. Where one is not — because verifying it
//! would need a multi-step model interaction this harness does not script, a real external
//! program (vim, htop), or an effect (color, animation) that a literal string match cannot
//! distinguish from its absence — that is a row in the report explaining why, not a
//! skipped row silently absent from it.
//!
//! Same ownership rule as `extension_compatibility.rs`: this file, `tests/pty/drive.py`,
//! and their fixtures are the only things owned here. It does not, and must not, edit
//! `crates/micro-extensions/host/**`, `crates/micro-extensions/src/host.rs`, or
//! `host/sdk/*.ts`.

mod support;

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use support::FakeApi;
use support::Fixture;
use support::Reply;

fn drive_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pty/drive.py")
}

fn vendored_extensions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions")
}

/// One interactive behavior worth checking, and the literal evidence that it happened.
struct InteractiveCheck {
    /// The extension under `examples/extensions/<name>.ts`.
    name: &'static str,
    /// What has to be true of the session before the probe means anything — most often
    /// that a prompt was already sent and answered, since a fresh session has no turns.
    setup: &'static [&'static str],
    /// The keystroke batch this behavior itself needs, sent after `setup`.
    keys: &'static [&'static str],
    /// How long to hold the session open, generous enough for Bun's own startup plus
    /// whatever this check drives.
    wait_secs: f64,
    /// Text that must reach the screen for the check to count as verified.
    probe: &'static str,
    /// What finding `probe` is evidence of, printed in the report.
    means: &'static str,
}

const INTERACTIVE_CHECKS: &[InteractiveCheck] = &[
    // Renders on session_start alone: no keys needed beyond letting the TUI open.
    InteractiveCheck {
        name: "widget-placement",
        setup: &[],
        keys: &[],
        wait_secs: 4.0,
        probe: "Above editor widget",
        means: "ctx.ui.setWidget() reached the screen from session_start",
    },
    InteractiveCheck {
        name: "custom-header",
        setup: &[],
        keys: &[],
        wait_secs: 4.0,
        probe: "shitty coding agent",
        means: "ctx.ui.setHeader() replaced the built-in header, and ctx.mode reported \"tui\"",
    },
    InteractiveCheck {
        name: "status-line",
        setup: &[],
        keys: &[],
        wait_secs: 4.0,
        probe: "Ready",
        means: "ctx.ui.setStatus() reached the footer from session_start",
    },
    InteractiveCheck {
        name: "modal-editor",
        setup: &[],
        keys: &[],
        wait_secs: 4.0,
        probe: "NORMAL",
        means: "ctx.ui.setEditorComponent() replaced the built-in editor with one that draws a mode indicator",
    },
    // One command, no model turn.
    InteractiveCheck {
        name: "working-indicator",
        setup: &[],
        keys: &["/working-indicator\r"],
        wait_secs: 5.0,
        probe: "custom spinner",
        means: "the registered command ran, read the extension's own state, and ctx.ui.notify() reached the screen",
    },
    InteractiveCheck {
        name: "hidden-thinking-label",
        setup: &[],
        keys: &["/thinking-label micro-probe-9f2c\r"],
        wait_secs: 5.0,
        probe: "micro-probe-9f2c",
        means: "the registered command read its argument and ctx.ui.notify() echoed it back",
    },
    InteractiveCheck {
        name: "message-renderer",
        setup: &[],
        keys: &["/status micro-probe-7a31\r"],
        wait_secs: 5.0,
        probe: "micro-probe-7a31",
        means: "registerMessageRenderer's custom renderer drew the message pi.sendMessage() sent",
    },
    // Needs a full turn: a prompt sent, a reply streamed back, turn_end fired.
    InteractiveCheck {
        name: "status-line",
        setup: &["say hi\r"],
        keys: &[],
        wait_secs: 7.0,
        probe: "Turn 1 complete",
        means: "turn_start and turn_end both fired and both reached ctx.ui.setStatus()",
    },
];

/// Names covered by [`INTERACTIVE_CHECKS`] but not marked as run twice in the report —
/// `status-line` appears above under two different checks (session_start, then a full
/// turn), and should only be counted once toward "how many of the 23 are covered."
fn distinct_names_covered() -> std::collections::BTreeSet<&'static str> {
    INTERACTIVE_CHECKS.iter().map(|check| check.name).collect()
}

/// Every terminal-only extension `extension_compatibility.rs` flagged, and why the ones
/// without a matching [`InteractiveCheck`] above are not covered here.
const NOT_COVERED: &[(&str, &str)] = &[
    (
        "question",
        "ctx.ui.select() is called from inside a tool the model has to call first; scripting that tool call through the fake provider while also driving pty timing was not built in this pass",
    ),
    (
        "questionnaire",
        "same shape as question.ts: the UI opens from a model-triggered flow, not from session_start or a plain command",
    ),
    (
        "qna",
        "ctx.ui.setEditorText() runs against the last assistant response; needs a scripted turn plus the exact trigger command, not attempted here",
    ),
    (
        "timed-confirm",
        "ctx.ui.confirm()/select() are demonstrated from a command handler this pass did not trace closely enough to script blind",
    ),
    (
        "rpc-demo",
        "exercises RPC-supported UI methods specifically — the interesting half of this one is micro's --rpc protocol, not a pty, and belongs with extension_compatibility.rs's approach instead",
    ),
    (
        "rainbow-editor",
        "its effect is a color animation on the substring \"ultrathink\" as it is typed; a literal-string probe cannot tell a colorized render from a plain one",
    ),
    (
        "titlebar-spinner",
        "writes the terminal title via an OSC sequence keyed to agent_start/agent_end, not to visible screen content; verifying it needs the same turn-driving as status-line's second check plus title-specific OSC parsing not built here",
    ),
    (
        "snake",
        "opens a live-updating game component; confirming a first frame draws needs source-level knowledge of its board rendering this pass did not go deep enough to script confidently",
    ),
    ("tic-tac-toe", "same shape as snake.ts: a live game component, not attempted here"),
    ("space-invaders", "same shape as snake.ts: a live game component, not attempted here"),
    (
        "doom-overlay",
        "a real-time overlay reading WAD assets and rendering at 35 FPS; well outside what a literal-string probe in a few seconds of pty time can responsibly claim to verify",
    ),
    (
        "overlay-test",
        "compositing edge cases across many inline inputs; team-lead's own pty pass already established that headers, footers, and widgets genuinely render, which is the same rendering path this extension exercises more elaborately",
    ),
    ("overlay-qa-tests", "same reasoning as overlay-test.ts: broad compositing QA, not a single behavior a short probe can characterize"),
    (
        "interactive-shell",
        "runs a real external program (vim, htop) inside the session; not attempted, since neither is guaranteed present and a missing binary would be misread as an extension failure",
    ),
    (
        "entry-renderer",
        "documented as TUI-only session *entry* rendering — its trigger (viewing session history) was not identified precisely enough in this pass to script",
    ),
];

/// Strip ANSI/C1 control sequences (CSI and OSC, plus lone two-byte escapes) so a probe
/// string can be found the way a person reading the screen would find it. This is not a
/// terminal emulator — cursor movement is not replayed, so text is matched in stream order
/// rather than final screen position, the same limitation a raw "strip escapes and grep"
/// pass has however it is implemented.
fn strip_ansi(raw: &[u8]) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == 0x1b && i + 1 < raw.len() {
            match raw[i + 1] {
                b'[' => {
                    // CSI: ESC [ ... final byte in 0x40..=0x7e.
                    let mut j = i + 2;
                    while j < raw.len() && !(0x40..=0x7e).contains(&raw[j]) {
                        j += 1;
                    }
                    i = (j + 1).min(raw.len());
                }
                b']' => {
                    // OSC: ESC ] ... terminated by BEL or ESC \.
                    let mut j = i + 2;
                    while j < raw.len()
                        && raw[j] != 0x07
                        && !(raw[j] == 0x1b && j + 1 < raw.len() && raw[j + 1] == b'\\')
                    {
                        j += 1;
                    }
                    i = if j < raw.len() && raw[j] == 0x07 {
                        j + 1
                    } else {
                        (j + 2).min(raw.len())
                    };
                }
                _ => i += 2,
            }
        } else if raw[i] < 0x20 && raw[i] != b'\n' && raw[i] != b'\t' && raw[i] != b'\r' {
            i += 1;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A `python3 tests/pty/drive.py -- <micro binary> <args>` command carrying exactly the
/// environment and working directory `fixture.micro()` already set up — including whatever
/// it removed, read back through `Command`'s own introspection rather than duplicated by
/// hand, so this never drifts from what `extension_compatibility.rs` and `cli.rs` rely on.
fn pty_command(fixture: &Fixture, micro_args: &[&str]) -> Command {
    let base = fixture.micro();
    let mut command = Command::new("python3");
    command.arg(drive_script());
    command.arg("--");
    command.arg(base.get_program());
    if let Some(dir) = base.get_current_dir() {
        command.current_dir(dir);
    }
    for (key, value) in base.get_envs() {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }
    command.args(micro_args);
    command
}

/// Run one check: install the extension, open a session in a pty, drive whatever setup and
/// keys it needs, and report whether the probe reached the screen.
fn run_check(check: &InteractiveCheck) -> Result<(), String> {
    let api = FakeApi::start([Reply::text("hello from the fake provider")]);
    let fixture = Fixture::new(&api);

    let source = vendored_extensions_dir().join(format!("{}.ts", check.name));
    let content = std::fs::read_to_string(&source)
        .unwrap_or_else(|error| panic!("read {}: {error}", source.display()));
    fixture.write(&format!(".micro/extensions/{}.ts", check.name), &content);

    let mut command = pty_command(&fixture, &["-m", "test"]);
    let batches: Vec<&str> = check.setup.iter().chain(check.keys.iter()).copied().collect();
    command.env("KEYS", batches.join("~~"));
    command.env("WAIT", check.wait_secs.to_string());
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let output = command
        .output()
        .map_err(|error| format!("could not run the pty driver: {error}"))?;
    if !output.status.success() && output.stdout.is_empty() {
        return Err(format!(
            "the pty driver itself failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let screen = strip_ansi(&output.stdout);
    if screen.contains(check.probe) {
        Ok(())
    } else {
        Err(format!("\"{}\" never reached the screen", check.probe))
    }
}

/// Verifies the interactive behavior of every terminal-only extension this harness can
/// responsibly probe, and reports the rest with the specific reason they are not covered.
///
/// Always passes, same as `extension_compatibility.rs`'s report: an extension that draws
/// nothing yet is the finding, not a defect in the harness. Run with `--nocapture` to read
/// the table.
#[test]
fn interactive_extensions_behave() {
    if micro_extensions::which_bun().is_none() {
        eprintln!("skipped: bun is not on the path, so nothing here could load anyway");
        return;
    }
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipped: python3 is not on the path, and the pty driver needs it");
        return;
    }

    let mut lines = vec![
        "| extension | check | result |".to_string(),
        "|---|---|---|".to_string(),
    ];
    let mut verified = 0usize;
    let mut failed = 0usize;

    for check in INTERACTIVE_CHECKS {
        let result = match run_check(check) {
            Ok(()) => {
                verified += 1;
                format!("OK — {}", check.means)
            }
            Err(reason) => {
                failed += 1;
                format!("FAIL — {reason}")
            }
        };
        let label = if check.setup.is_empty() {
            "startup"
        } else {
            "after a turn"
        };
        lines.push(format!("| {} | {label} | {result} |", check.name));
    }

    lines.push(String::new());
    lines.push(format!(
        "{verified} of {} behavior checks confirmed, across {} distinct extensions; {failed} did not.",
        INTERACTIVE_CHECKS.len(),
        distinct_names_covered().len()
    ));
    lines.push(String::new());
    lines.push("Not covered, and why:".to_string());
    for (name, reason) in NOT_COVERED {
        lines.push(format!("- {name}: {reason}"));
    }

    eprintln!("\n{}", lines.join("\n"));
}
