//! What a session says it cost, against what the provider said it cost.
//!
//! The unit tests around the bill prove the shares of a turn add up to it. This proves the
//! number they add up to is the right one: the sessions here are real runs of the binary,
//! the usage is what a real provider reports on the wire, and the prices are the ones in
//! the catalog the run reads.

mod support;

use serde_json::json;
use serde_json::Value;
use support::FakeApi;
use support::Fixture;
use support::Reply;

/// The rates the fixture's model is billed at, in dollars per million tokens.
///
/// Chosen so every figure below divides exactly: a bill that came out to a repeating
/// fraction would prove the arithmetic and not much else.
const INPUT: f64 = 3.0;
const OUTPUT: f64 = 15.0;
const CACHE_READ: f64 = 0.3;

/// A reply that also says what the provider billed for it.
///
/// `prompt` counts the cached tokens too, which is how a real service reports it and why
/// the fresh input a bill charges at the full rate is the difference between the two.
fn billed(mut chunks: Vec<Value>, prompt: u64, cached: u64, completion: u64) -> Reply {
    chunks.push(json!({
        "choices": [],
        "usage": {
            "prompt_tokens": prompt,
            "completion_tokens": completion,
            "prompt_tokens_details": { "cached_tokens": cached },
        },
    }));
    Reply::Sse(chunks)
}

fn answer(text: &str, prompt: u64, cached: u64, completion: u64) -> Reply {
    billed(
        vec![support::text_delta(text), support::finish("stop")],
        prompt,
        cached,
        completion,
    )
}

fn asks_for_a_tool(prompt: u64, cached: u64, completion: u64) -> Reply {
    billed(
        vec![
            support::tool_call_open(0, "call_1", "ls"),
            support::tool_call_arguments(0, &json!({ "path": "." }).to_string()),
            support::finish("tool_calls"),
        ],
        prompt,
        cached,
        completion,
    )
}

/// Give the fixture's model a price, so there is something to bill.
///
/// The fixture writes a model with no cost at all, which every other test wants: a run
/// that reports nothing is a run nothing distracts from. A bill has nothing to say about
/// one, so this replaces the file with the same model priced.
fn priced(fixture: &Fixture, api: &FakeApi) {
    std::fs::write(
        fixture.home().join("models.json"),
        json!({
            "providers": {
                "openai": {
                    "base_url": api.base_url(),
                    "api": "openai-completions",
                    "models": [{
                        "id": "test-model",
                        "name": "Test Model",
                        "context_window": 200000,
                        "max_output_tokens": 4096,
                        "aliases": ["test"],
                        "cost": {
                            "input": INPUT,
                            "output": OUTPUT,
                            "cache_read": CACHE_READ,
                            "cache_write": 3.75,
                        },
                    }],
                },
            },
        })
        .to_string(),
    )
    .expect("write a priced models.json");
}

/// Write a session's log and sidecar straight to disk, in the format `docs/ledger.md`
/// documents, and answer with its id.
///
/// A session put there by hand rather than by a run, so `micro bill` can be read against a
/// ledger whose every number is known in advance. The turns are what a two-turn session
/// records: a prompt, a request describing what it was assembled from, what the provider
/// billed, and the answer.
fn written(fixture: &Fixture, turns: &[(u64, u64, u64, u64)]) -> String {
    let id = format!("{}", 1_786_000_000_000u64 + std::process::id() as u64);
    let sessions = fixture.home().join("sessions");
    std::fs::create_dir_all(&sessions).expect("the session directory");

    let mut lines: Vec<String> = Vec::new();
    let mut seq = 0;
    let mut entry = 0;
    for (turn, prompt, cached, completion) in turns.iter().copied() {
        entry += 1;
        lines.push(
            json!({
                "id": entry.to_string(),
                "parent_id": (entry > 1).then(|| (entry - 1).to_string()),
                "timestamp": 1_786_000_000_000u64,
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "hello" }],
                    "timestamp": 1_786_000_000_000u64,
                },
            })
            .to_string(),
        );
        seq += 1;
        lines.push(
            json!({
                "v": 1, "seq": seq, "ts": 1_786_000_000_000u64,
                "event": {
                    "type": "turn_request", "turn": turn,
                    "provider": "openai", "model": "test-model",
                    "prefix_hash": "aa", "request_hash": "bb",
                    // Named but not written: a blob that is not there leaves the tools out
                    // of the sharing rather than failing the bill.
                    "tools_blob": "cc", "model_blob": "dd",
                    "prefix_spans": [
                        { "source": "system_prompt", "bytes": 400, "hash": "ee" },
                        { "source": "project_instructions", "bytes": 100, "hash": "ff" },
                    ],
                    "message_entry_ids": [entry.to_string()],
                    "attempt": 1,
                },
            })
            .to_string(),
        );
        seq += 1;
        lines.push(
            json!({
                "v": 1, "seq": seq, "ts": 1_786_000_000_000u64,
                "event": {
                    "type": "turn_usage", "turn": turn,
                    "usage": {
                        "input": prompt - cached, "output": completion,
                        "cache_read": cached, "cache_write": 0,
                    },
                    "stop_reason": "stop", "provider": "openai", "model": "test-model",
                },
            })
            .to_string(),
        );
        entry += 1;
        lines.push(
            json!({
                "id": entry.to_string(),
                "parent_id": (entry - 1).to_string(),
                "timestamp": 1_786_000_000_000u64,
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "done" }],
                    "provider": "openai", "model": "test-model",
                    "usage": {
                        "input": prompt - cached, "output": completion,
                        "cache_read": cached, "cache_write": 0,
                    },
                    "stop_reason": "stop", "timestamp": 1_786_000_000_000u64,
                },
            })
            .to_string(),
        );
    }

    std::fs::write(
        sessions.join(format!("{id}.jsonl")),
        format!("{}\n", lines.join("\n")),
    )
    .expect("write the session log");
    std::fs::write(
        sessions.join(format!("{id}.meta.json")),
        json!({
            "v": 1, "id": id,
            "created_at": 1_786_000_000_000u64, "updated_at": 1_786_000_000_000u64,
            "workspace": fixture.workspace(),
            "model_id": "openai/test-model",
            "title": "hello", "message_count": entry,
        })
        .to_string(),
    )
    .expect("write the session sidecar");
    id
}

/// The one session this fixture recorded, by id.
fn session_id(fixture: &Fixture) -> String {
    let directory = fixture.home().join("sessions");
    let mut ids: Vec<String> = std::fs::read_dir(&directory)
        .expect("the session directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "jsonl"))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .collect();
    ids.sort();
    ids.pop().expect("a session was recorded")
}

/// Every dollar amount on one line of a report, in the order they appear.
fn amounts(line: &str) -> Vec<f64> {
    line.split('$')
        .skip(1)
        .filter_map(|rest| {
            let digits: String = rest
                .chars()
                .take_while(|character| character.is_ascii_digit() || *character == '.')
                .collect();
            digits.parse().ok()
        })
        .collect()
}

/// A turn's own total, and the shares printed underneath it.
struct Block {
    total: f64,
    shares: Vec<f64>,
}

/// The report read back as the blocks it is made of.
fn blocks(report: &str) -> Vec<Block> {
    let mut read: Vec<Block> = Vec::new();
    for line in report.lines() {
        let Some(amount) = amounts(line).last().copied() else {
            continue;
        };
        match line.starts_with("  ") {
            true => {
                if let Some(block) = read.last_mut() {
                    block.shares.push(amount);
                }
            }
            false if line.starts_with("turn ") => read.push(Block {
                total: amount,
                shares: Vec::new(),
            }),
            false => {}
        }
    }
    read
}

/// `micro bill` against a ledger written by hand: every figure in it is known in advance,
/// so what the subcommand prints can be checked rather than merely parsed.
///
/// With no session named it bills the latest one from this workspace, which is the form
/// somebody actually types.
#[test]
fn the_bill_subcommand_reads_a_recorded_ledger() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);
    priced(&fixture, &api);
    let id = written(&fixture, &[(1, 1_000, 0, 200), (2, 1_500, 1_000, 100)]);

    let billed = fixture.micro_run(&["bill"]);
    billed.expect_success("micro bill");
    let report = &billed.stdout;
    assert!(
        report.contains(&format!("Bill for session {id}")),
        "{report}"
    );

    let read = blocks(report);
    assert_eq!(read.len(), 2, "one block per turn: {report}");
    for (index, block) in read.iter().enumerate() {
        let shared: f64 = block.shares.iter().sum();
        assert!(
            (shared - block.total).abs() < 5e-6,
            "turn {} shares add up to {shared}, not {}: {report}",
            index + 1,
            block.total
        );
    }

    let first = (1_000.0 * INPUT + 200.0 * OUTPUT) / 1e6;
    let second = (500.0 * INPUT + 100.0 * OUTPUT + 1_000.0 * CACHE_READ) / 1e6;
    assert!((read[0].total - first).abs() < 5e-7, "turn 1: {report}");
    assert!((read[1].total - second).abs() < 5e-7, "turn 2: {report}");
    assert!(
        report.contains(&format!("${:.6}", first + second)),
        "the total should be the two turns: {report}"
    );
    for named in ["system_prompt", "project_instructions", "user", "model"] {
        assert!(report.contains(named), "no {named} line: {report}");
    }

    let diffed = fixture.micro_run(&["bill", &id, "--diff", "2"]);
    diffed.expect_success("micro bill --diff");
    let diff = &diffed.stdout;
    assert!(diff.starts_with("Turn 2 of session "), "{diff}");
    assert!(diff.contains("What it added"), "{diff}");
    assert!(
        diff.contains(&format!("${second:.6}")),
        "what this turn came to: {diff}"
    );
    assert!(diff.contains("running total before"), "{diff}");
    assert!(diff.contains(&format!("${first:.6}")), "{diff}");
    assert!(
        diff.contains(&format!("${:.6}", first + second)),
        "and after it: {diff}"
    );
    assert!(diff.contains("came back out of cache"), "{diff}");

    let missing = fixture.micro_run(&["bill", &id, "--diff", "9"]);
    missing.expect_failure("a turn nobody billed");
}

/// The bill's arithmetic, end to end: every line item adds up to its turn, every turn adds
/// up to the total, and the total is what the provider's own numbers price out to.
///
/// The two turns are deliberately different shapes — one that asked for a tool and one that
/// answered, one that paid for its whole prompt and one that read most of it back out of
/// cache — because a bill that only ever sees one shape proves nothing about the other.
#[test]
fn a_bill_adds_up_to_what_the_provider_reported() {
    let api = FakeApi::start([
        asks_for_a_tool(1_000, 0, 200),
        answer("done", 1_500, 1_000, 100),
    ]);
    let fixture = Fixture::new(&api);
    priced(&fixture, &api);
    fixture.write("AGENTS.md", "Run the linter before you finish.");

    fixture
        .print(&["-m", "test", "list the files"])
        .expect_success("micro --print");
    assert_eq!(api.request_count(), 2, "a tool call means a second turn");

    let billed = fixture.micro_run(&["bill", &session_id(&fixture)]);
    billed.expect_success("micro bill");
    let report = &billed.stdout;

    let read = blocks(report);
    assert_eq!(read.len(), 2, "one block per turn: {report}");
    for (index, block) in read.iter().enumerate() {
        assert!(
            !block.shares.is_empty(),
            "turn {} was not itemized: {report}",
            index + 1
        );
        let shared: f64 = block.shares.iter().sum();
        assert!(
            (shared - block.total).abs() < 5e-6,
            "turn {} shares add up to {shared}, not {}: {report}",
            index + 1,
            block.total
        );
    }

    // What the two turns should come to, worked out from the numbers the fake provider
    // reported and the rates the catalog carries, with nothing read out of the report.
    let first = (1_000.0 * INPUT + 200.0 * OUTPUT) / 1e6;
    let second = (500.0 * INPUT + 100.0 * OUTPUT + 1_000.0 * CACHE_READ) / 1e6;
    assert!((read[0].total - first).abs() < 5e-7, "turn 1: {report}");
    assert!((read[1].total - second).abs() < 5e-7, "turn 2: {report}");
    assert!(
        report.contains(&format!("${:.6}", first + second)),
        "the total should be the two turns: {report}"
    );
}

/// Every source that put bytes into a turn earns a line, named the way the ledger names it.
#[test]
fn a_bill_names_where_the_money_went() {
    let api = FakeApi::start([answer("done", 1_000, 0, 200)]);
    let fixture = Fixture::new(&api);
    priced(&fixture, &api);
    fixture.write("AGENTS.md", "Run the linter before you finish.");

    fixture
        .print(&["-m", "test", "hello"])
        .expect_success("micro --print");

    let billed = fixture.micro_run(&["bill", &session_id(&fixture)]);
    billed.expect_success("micro bill");
    let report = &billed.stdout;

    for named in ["system_prompt", "project_instructions", "user", "model"] {
        assert!(report.contains(named), "no {named} line: {report}");
    }
    // A tool is charged for being offered as well as for what it answers, so it is on the
    // bill of a turn that never called one.
    assert!(report.contains("tool:read"), "no tool lines: {report}");
    assert!(
        report.contains("always add up to the turn"),
        "the report should say what is exact and what is an estimate: {report}"
    );
}

/// `--diff` answers a different question from the bill: not what the session cost, but
/// what one turn added to it, and what about that turn made it cost that.
#[test]
fn the_diff_of_a_turn_says_what_it_added_and_why() {
    let api = FakeApi::start([
        asks_for_a_tool(1_000, 0, 200),
        answer("done", 1_500, 1_000, 100),
    ]);
    let fixture = Fixture::new(&api);
    priced(&fixture, &api);

    fixture
        .print(&["-m", "test", "list the files"])
        .expect_success("micro --print");

    let diffed = fixture.micro_run(&["bill", &session_id(&fixture), "--diff", "2"]);
    diffed.expect_success("micro bill --diff");
    let report = &diffed.stdout;

    assert!(report.starts_with("Turn 2 of session "), "{report}");

    let before = (1_000.0 * INPUT + 200.0 * OUTPUT) / 1e6;
    let added = (500.0 * INPUT + 100.0 * OUTPUT + 1_000.0 * CACHE_READ) / 1e6;
    assert!(report.contains("running total before"), "{report}");
    assert!(
        report.contains(&format!("${before:.6}")),
        "what the session had spent before this turn: {report}"
    );
    assert!(report.contains("running total after"), "{report}");
    assert!(
        report.contains(&format!("${:.6}", before + added)),
        "and after it: {report}"
    );

    assert!(report.contains("Why it cost that"), "{report}");
    assert!(
        report.contains("came back out of cache"),
        "the second turn read two thirds of its prompt from cache, which is most of why it \
         cost what it did: {report}"
    );
    assert!(
        report.contains("more bytes into the prompt than at turn 1"),
        "and the tool's result is what grew: {report}"
    );
}

/// `/bill` is the same reading from inside a session, so a run that is already open does
/// not have to be left to find out what it has spent.
#[test]
fn the_bill_command_reports_on_the_session_it_is_run_in() {
    let api = FakeApi::start([answer("done", 1_000, 0, 200)]);
    let fixture = Fixture::new(&api);
    priced(&fixture, &api);

    fixture
        .print(&["-m", "test", "hello"])
        .expect_success("micro --print");
    let id = session_id(&fixture);

    let asked = fixture.print(&["-m", "test", "--resume", &id, "/bill"]);
    asked.expect_success("micro --print /bill");
    let report = &asked.stdout;

    assert!(
        report.contains(&format!("Bill for session {id}")),
        "the session it was run in: {report}"
    );
    let expected = (1_000.0 * INPUT + 200.0 * OUTPUT) / 1e6;
    assert!(
        report.contains(&format!("${expected:.6}")),
        "and what it has cost: {report}"
    );
    assert_eq!(
        api.request_count(),
        1,
        "a slash command is run rather than sent to the model"
    );
}

/// A ceiling stops the run at the first turn boundary past it, says so where an error
/// would be said, and leaves the reason on the ledger.
///
/// The budget here is smaller than one turn on purpose: what is being proved is that the
/// check happens and lands, not where the threshold sits.
#[test]
fn a_session_over_its_budget_stops_and_says_so() {
    let api = FakeApi::start([answer("done", 1_000, 0, 200)]);
    let fixture = Fixture::new(&api);
    priced(&fixture, &api);

    let run = fixture.print(&["-m", "test", "--budget", "0.00001", "hello"]);
    assert!(
        !run.status.success(),
        "a run that stopped short should say so in its status: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("Stopped: this session has spent"),
        "and in what it printed: {}",
        run.stderr
    );
    assert!(
        run.stdout.contains("done"),
        "the answer that was paid for is still shown: {}",
        run.stdout
    );

    let exported = fixture.micro_run(&["sessions", "export", &session_id(&fixture)]);
    exported.expect_success("micro sessions export");
    let stopped: Vec<Value> = exported
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line["event"]["type"] == "budget_stop")
        .collect();
    assert_eq!(stopped.len(), 1, "one stop, recorded: {}", exported.stdout);
    assert_eq!(stopped[0]["event"]["limit"], 0.00001);
    assert!(
        stopped[0]["event"]["spent"].as_f64().unwrap_or_default() > 0.0,
        "with what it had spent: {}",
        exported.stdout
    );
}

/// Reopening a session that is already over its ceiling has to work. The stop is a fact
/// about what a turn may do next, not a lock on the file: raising the limit lifts it, and
/// the conversation carries on from where it stopped.
#[test]
fn raising_the_budget_lets_a_stopped_session_carry_on() {
    let api = FakeApi::start([
        answer("first", 1_000, 0, 200),
        answer("second", 1_200, 0, 100),
    ]);
    let fixture = Fixture::new(&api);
    priced(&fixture, &api);

    fixture
        .print(&["-m", "test", "--budget", "0.00001", "hello"])
        .expect_failure("a run over its budget");
    let id = session_id(&fixture);

    let carried = fixture.print(&["-m", "test", "--resume", &id, "--budget", "10", "again"]);
    carried.expect_success("micro --print with room to spend");
    assert!(
        carried.stdout.contains("second"),
        "the resumed run should have answered: {}",
        carried.stdout
    );
    assert_eq!(api.request_count(), 2, "both prompts reached the provider");
}

/// A ceiling covers the session rather than each run of it, so what earlier runs spent
/// counts against it — otherwise reopening a session would hand it the whole budget again.
#[test]
fn what_earlier_runs_spent_counts_against_the_budget() {
    let api = FakeApi::start([
        answer("first", 1_000, 0, 200),
        answer("second", 1_200, 0, 100),
    ]);
    let fixture = Fixture::new(&api);
    priced(&fixture, &api);

    // One turn costs $0.006, so a six-tenths-of-a-cent ceiling survives the first run and
    // not the second.
    let ceiling = "0.0065";
    fixture
        .print(&["-m", "test", "--budget", ceiling, "hello"])
        .expect_success("the first run stays under the ceiling");
    let id = session_id(&fixture);

    let carried = fixture.print(&["-m", "test", "--resume", &id, "--budget", ceiling, "again"]);
    assert!(
        !carried.status.success(),
        "the second run should have taken the session over: {}",
        carried.stderr
    );
    assert!(
        carried.stderr.contains("Stopped: this session has spent"),
        "{}",
        carried.stderr
    );
}
