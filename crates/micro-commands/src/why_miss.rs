//! Why a turn paid for a prompt the provider already had.
//!
//! A cache miss is normally invisible: the bill is larger and nothing says what moved. The
//! ledger recorded what each turn opened with and every change to it, so the question is
//! answerable exactly — which span of the prompt differs, what it used to say, and which
//! recorded change is responsible.

use crate::CommandContext;
use crate::CommandOutcome;
use micro_session::ReconstructedTurn;
use micro_session::SessionStore;
use micro_types::LedgerEvent;
use micro_types::PrefixSpan;
use std::collections::HashSet;

/// How many message boundaries a provider will hold a cache breakpoint on.
///
/// Providers place a small, fixed number of them, and the last ones sit on the most recent
/// turns; everything before the oldest breakpoint is only reused when nothing in front of
/// it moved. A turn that added more messages than this since the last request can miss even
/// with an identical prefix, so it is worth saying so.
const BREAKPOINT_WINDOW: usize = 2;

/// The most lines of prompt this will compare. Beyond it the comparison is quadratic in
/// the size of the prompt, which is a bad trade for output nobody would read anyway.
const MAX_DIFF_LINES: usize = 1_000;

#[derive(Clone)]
struct Request {
    turn: u64,
    seq: u64,
    path: Vec<String>,
    prefix_hash: String,
}

/// `/why-miss [turn]`.
pub(crate) async fn command(
    argument: Option<&str>,
    context: &CommandContext<'_>,
) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error(
            "nothing to explain: this conversation is not being recorded",
        );
    };

    let turn = match argument.map(str::trim).filter(|turn| !turn.is_empty()) {
        Some(argument) => match argument.parse::<u64>() {
            Ok(turn) => Some(turn),
            Err(_) => return CommandOutcome::error(format!("{argument} is not a turn number")),
        },
        None => None,
    };

    match why_miss(context.sessions, session_id, turn).await {
        Ok(explanation) => CommandOutcome::inspect("Cache diagnosis", explanation),
        Err(reason) => CommandOutcome::error(reason),
    }
}

/// Why the prefix of one turn was, or was not, the prefix of the turn before it.
///
/// `turn` names which one. Without one, the latest completed turn on the current branch
/// whose prefix differs from its parent is used. What comes back is the whole explanation, so the caller
/// showing it — a terminal, a subcommand — decides nothing about it.
pub async fn why_miss(
    store: &SessionStore,
    session_id: &str,
    turn: Option<u64>,
) -> Result<String, String> {
    let loaded = store
        .load(session_id)
        .await
        .map_err(|error| format!("cannot read session {session_id}: {error}"))?;

    let mut requests: Vec<Request> = Vec::new();
    let mut changes: Vec<(u64, LedgerEvent)> = Vec::new();
    let mut completed = HashSet::new();
    for recorded in loaded.session.events() {
        match &recorded.event {
            LedgerEvent::TurnRequest {
                turn,
                message_entry_ids,
                prefix_hash,
                ..
            } => {
                // A turn re-issued after a transient failure was recorded once per attempt.
                // The last attempt is the request that stands.
                let request = Request {
                    turn: *turn,
                    seq: recorded.seq,
                    path: message_entry_ids.clone(),
                    prefix_hash: prefix_hash.clone(),
                };
                match requests.iter_mut().find(|known| known.turn == *turn) {
                    Some(known) => *known = request,
                    None => requests.push(request),
                }
            }
            LedgerEvent::TurnUsage { turn, .. } => {
                completed.insert(*turn);
            }
            LedgerEvent::PrefixChanged { .. }
            | LedgerEvent::Compaction { .. }
            | LedgerEvent::HeadMoved { .. } => changes.push((recorded.seq, recorded.event.clone())),
            _ => {}
        }
    }

    if requests.is_empty() {
        return Err(format!(
            "session {session_id} recorded no requests. A session written before the ledger \
             existed holds its conversation and nothing else."
        ));
    }

    let current_path = loaded.session.tree().path_entry_ids();
    let wanted = match turn {
        Some(turn) => turn,
        None => requests
            .iter()
            .filter(|request| {
                path_is_prefix(&request.path, &current_path) && completed.contains(&request.turn)
            })
            .rev()
            .find(|request| {
                parent_of(&requests, request)
                    .is_some_and(|parent| parent.prefix_hash != request.prefix_hash)
            })
            .or_else(|| {
                requests.iter().rev().find(|request| {
                    path_is_prefix(&request.path, &current_path)
                        && completed.contains(&request.turn)
                })
            })
            .map(|request| request.turn)
            .ok_or_else(|| format!("session {session_id} has no completed turn on this branch"))?,
    };
    let Some(request) = requests.iter().find(|request| request.turn == wanted) else {
        return Err(format!("session {session_id} has no turn {wanted}"));
    };

    let mut out = format!("session {session_id}  turn {wanted}\n\n");

    let Some(parent) = parent_of(&requests, request) else {
        out.push_str(
            "This is the session's first request, so there was nothing cached to reuse. The \
             prompt it sent is what every later turn is compared against.\n",
        );
        return Ok(out);
    };
    let previous = parent.turn;
    let previous_seq = parent.seq;
    let wanted_seq = request.seq;
    // Only what happened between the two requests can explain the difference between them.
    let between: Vec<&(u64, LedgerEvent)> = changes
        .iter()
        .filter(|(seq, _)| *seq > previous_seq && *seq < wanted_seq)
        .collect();

    let before = store
        .reconstruct_turn(session_id, previous)
        .await
        .map_err(|error| format!("cannot read turn {previous}: {error}"))?;
    let after = store
        .reconstruct_turn(session_id, wanted)
        .await
        .map_err(|error| format!("cannot read turn {wanted}: {error}"))?;

    match before.prefix_hash == after.prefix_hash {
        true => held(&mut out, previous, &before, &after, &between),
        false => moved(&mut out, wanted, previous, &before, &after, &between),
    }
    Ok(out)
}

/// The model request this request continues. Branches are paths through message entry ids,
/// so the parent is the latest earlier request whose path is a prefix of this one.
fn parent_of<'a>(requests: &'a [Request], request: &Request) -> Option<&'a Request> {
    requests
        .iter()
        .filter(|candidate| {
            candidate.turn != request.turn
                && candidate.seq < request.seq
                && path_is_prefix(&candidate.path, &request.path)
        })
        .max_by_key(|candidate| (candidate.path.len(), candidate.seq))
}

fn path_is_prefix(ancestor: &[String], descendant: &[String]) -> bool {
    ancestor.len() <= descendant.len() && descendant[..ancestor.len()] == ancestor[..]
}

/// The report for a turn whose prefix was byte for byte the one before it.
fn held(
    out: &mut String,
    previous: u64,
    before: &ReconstructedTurn,
    after: &ReconstructedTurn,
    between: &[&(u64, LedgerEvent)],
) {
    out.push_str(&format!(
        "The prefix held: {}\nEverything ahead of the conversation — {} bytes of system \
         prompt, {} tools — is what turn {previous} sent.\n",
        short(&after.prefix_hash),
        after.system_prompt.as_deref().unwrap_or_default().len(),
        after.tools.len(),
    ));

    let reasons = tail_reasons(previous, before, after, between);
    if reasons.is_empty() {
        out.push_str(
            "\nNothing recorded between the two turns could have moved the cache either: the \
             conversation only grew, by less than the breakpoint window.\n",
        );
        return;
    }

    out.push_str("\nThe conversation after it could still have missed:\n");
    for reason in &reasons {
        out.push_str(&format!("  - {reason}\n"));
    }
}

/// The report for a turn that opened with something else.
fn moved(
    out: &mut String,
    wanted: u64,
    previous: u64,
    before: &ReconstructedTurn,
    after: &ReconstructedTurn,
    between: &[&(u64, LedgerEvent)],
) {
    out.push_str(&format!(
        "The prefix changed between turn {previous} and turn {wanted}:\n  from  {}\n  to    \
         {}\n",
        short(&before.prefix_hash),
        short(&after.prefix_hash),
    ));

    let recorded: Vec<(u64, &str)> = between
        .iter()
        .filter_map(|(seq, event)| match event {
            LedgerEvent::PrefixChanged { reason, .. } => Some((*seq, reason.as_str())),
            _ => None,
        })
        .collect();

    if before.tools != after.tools {
        out.push_str(&format!("\n{}\n", tools_changed(before, after)));
    }

    let before_prompt = before.system_prompt.as_deref().unwrap_or_default();
    let after_prompt = after.system_prompt.as_deref().unwrap_or_default();
    if before_prompt != after_prompt {
        let (span, before_text, after_text) = changed_span(
            before_prompt,
            &before.prefix_spans,
            after_prompt,
            &after.prefix_spans,
        );
        match span {
            Some(source) => out.push_str(&format!("\nThe {source} span of the prompt changed.\n")),
            None => out.push_str("\nThe system prompt changed.\n"),
        }
        out.push('\n');
        for line in diff(&before_text, &after_text) {
            out.push_str(&format!("  {line}\n"));
        }
    }

    out.push('\n');
    match recorded.as_slice() {
        [] => out.push_str(
            "Nothing recorded says why. A change to the prompt or the tools that leaves no \
             prefix change behind it was made on the way to the provider rather than at a \
             turn boundary, which is the one thing this cannot account for.\n",
        ),
        recorded => {
            for (seq, reason) in recorded {
                out.push_str(&format!(
                    "The prefix changed because {}, recorded at seq {seq}.\n",
                    said(reason)
                ));
            }
        }
    }
}

/// What a recorded reason means, said as a sentence rather than as a tag.
fn said(reason: &str) -> String {
    match reason {
        "reload" => "the project's instructions and skills were read again (reload)".to_string(),
        "tools" => "the tools the model is told about were narrowed or widened (tools)".to_string(),
        extension if extension.starts_with("extension") => {
            format!("an extension replaced the system prompt ({extension})")
        }
        other => other.to_string(),
    }
}

/// Everything about the conversation, rather than the prefix, that could still cost a hit.
fn tail_reasons(
    previous: u64,
    before: &ReconstructedTurn,
    after: &ReconstructedTurn,
    between: &[&(u64, LedgerEvent)],
) -> Vec<String> {
    let mut reasons = Vec::new();

    for (seq, event) in between {
        match event {
            LedgerEvent::Compaction { kept, .. } => reasons.push(format!(
                "the conversation was summarized at seq {seq}, which replaced everything \
                 before the last {kept} messages — a provider caches from the front, so \
                 none of what came before is reusable",
            )),
            LedgerEvent::HeadMoved { entry_id } => reasons.push(format!(
                "the conversation was moved to another branch at seq {seq} (entry \
                 {entry_id}), so it no longer continues what was cached",
            )),
            _ => {}
        }
    }

    let grew = after.messages.len().saturating_sub(before.messages.len());
    if after.messages.len() < before.messages.len() {
        reasons.push(format!(
            "the conversation is {} messages shorter than at turn {previous}, so it is not \
             a continuation of what was sent then",
            before.messages.len() - after.messages.len()
        ));
    } else if grew > BREAKPOINT_WINDOW {
        reasons.push(format!(
            "{grew} messages were added since turn {previous}, and a provider holds a \
             breakpoint on only the last {BREAKPOINT_WINDOW} of them — everything past the \
             oldest breakpoint is charged as fresh input",
        ));
    }

    reasons
}

/// Which tools came and went, for a prefix whose tool definitions moved.
fn tools_changed(before: &ReconstructedTurn, after: &ReconstructedTurn) -> String {
    let named = |turn: &ReconstructedTurn| -> Vec<String> {
        turn.tools.iter().map(|tool| tool.name.clone()).collect()
    };
    let (had, has) = (named(before), named(after));

    let gone: Vec<String> = had
        .iter()
        .filter(|name| !has.contains(name))
        .cloned()
        .collect();
    let new: Vec<String> = has
        .iter()
        .filter(|name| !had.contains(name))
        .cloned()
        .collect();

    match (gone.is_empty(), new.is_empty()) {
        (true, true) => format!(
            "The same {} tools are offered, but one or more of their definitions changed.",
            has.len()
        ),
        (false, true) => format!("The tools no longer offer {}.", gone.join(", ")),
        (true, false) => format!("The tools now also offer {}.", new.join(", ")),
        (false, false) => format!(
            "The tools no longer offer {}, and now offer {}.",
            gone.join(", "),
            new.join(", ")
        ),
    }
}

/// The one stretch of the prompt that differs, and both sides of it.
///
/// Spans tile the prompt exactly, so a stretch can be cut back out of it by adding up the
/// lengths in front. When the two prompts were not built from the same parts — an extension
/// replaced one outright — there is no such stretch and the whole prompt is compared.
fn changed_span(
    before_prompt: &str,
    before_spans: &[PrefixSpan],
    after_prompt: &str,
    after_spans: &[PrefixSpan],
) -> (Option<String>, String, String) {
    let whole = || (None, before_prompt.to_string(), after_prompt.to_string());

    let same_shape = before_spans.len() == after_spans.len()
        && before_spans
            .iter()
            .zip(after_spans)
            .all(|(had, has)| had.source == has.source);
    if !same_shape {
        return whole();
    }

    let differing: Vec<usize> = before_spans
        .iter()
        .zip(after_spans)
        .enumerate()
        .filter(|(_, (had, has))| had.hash != has.hash)
        .map(|(index, _)| index)
        .collect();
    let [only] = differing.as_slice() else {
        return whole();
    };

    match (
        cut(before_prompt, before_spans, *only),
        cut(after_prompt, after_spans, *only),
    ) {
        (Some(had), Some(has)) => (
            Some(after_spans[*only].source.to_string()),
            had.to_string(),
            has.to_string(),
        ),
        _ => whole(),
    }
}

/// One span's own text, cut back out of the prompt it was joined into.
fn cut<'a>(prompt: &'a str, spans: &[PrefixSpan], index: usize) -> Option<&'a str> {
    let start: usize = spans[..index].iter().map(|span| span.bytes as usize).sum();
    let end = start + spans[index].bytes as usize;
    prompt.get(start..end)
}

/// A line-by-line comparison, in the shape a patch is read in.
///
/// Written here rather than taken from a library because what it has to do is small: two
/// stretches of a system prompt, compared once, for a person deciding what changed. The
/// longest common subsequence of the lines is what everything not in it is reported
/// against.
fn diff(before: &str, after: &str) -> Vec<String> {
    let had: Vec<&str> = before.lines().collect();
    let has: Vec<&str> = after.lines().collect();

    if had.len() > MAX_DIFF_LINES || has.len() > MAX_DIFF_LINES {
        return vec![format!(
            "({} lines became {} lines; too much to compare line by line)",
            had.len(),
            has.len()
        )];
    }

    // The length of the longest common subsequence of every pair of suffixes, filled from
    // the end so the walk below can always take the larger of the two ways forward.
    let mut common = vec![vec![0u32; has.len() + 1]; had.len() + 1];
    for left in (0..had.len()).rev() {
        for right in (0..has.len()).rev() {
            common[left][right] = match had[left] == has[right] {
                true => common[left + 1][right + 1] + 1,
                false => common[left + 1][right].max(common[left][right + 1]),
            };
        }
    }

    let mut lines = Vec::new();
    let (mut left, mut right) = (0, 0);
    while left < had.len() && right < has.len() {
        if had[left] == has[right] {
            lines.push(format!("  {}", had[left]));
            left += 1;
            right += 1;
        } else if common[left + 1][right] >= common[left][right + 1] {
            lines.push(format!("- {}", had[left]));
            left += 1;
        } else {
            lines.push(format!("+ {}", has[right]));
            right += 1;
        }
    }
    for line in &had[left..] {
        lines.push(format!("- {line}"));
    }
    for line in &has[right..] {
        lines.push(format!("+ {line}"));
    }

    around_the_changes(lines)
}

/// The changed lines and enough either side to place them, rather than the whole of two
/// prompts that are mostly the same.
fn around_the_changes(lines: Vec<String>) -> Vec<String> {
    const CONTEXT: usize = 2;

    let changed: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with('-') || line.starts_with('+'))
        .map(|(index, _)| index)
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }

    let mut kept = Vec::new();
    let mut last: Option<usize> = None;
    for index in 0..lines.len() {
        let near = changed
            .iter()
            .any(|change| index.abs_diff(*change) <= CONTEXT);
        if !near {
            continue;
        }
        if last.is_some_and(|last| index > last + 1) {
            kept.push("…".to_string());
        }
        kept.push(lines[index].clone());
        last = Some(index);
    }
    kept
}

/// A hash short enough to read, which is all a person comparing two of them needs.
fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diff_shows_the_line_that_changed_and_leaves_the_rest_alone() {
        let before = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight";
        let after = "one\ntwo\nthree\nFOUR\nfive\nsix\nseven\neight";

        let lines = diff(before, after);

        assert!(lines.contains(&"- four".to_string()));
        assert!(lines.contains(&"+ FOUR".to_string()));
        assert!(lines.contains(&"  three".to_string()), "with context");
        assert!(
            !lines.iter().any(|line| line.contains("eight")),
            "and without the lines nowhere near it: {lines:?}"
        );
    }

    #[test]
    fn two_identical_stretches_have_nothing_to_report() {
        assert!(diff("same\ntext", "same\ntext").is_empty());
    }

    #[test]
    fn an_added_line_is_shown_as_added() {
        let lines = diff("keep", "keep\nadded");
        assert_eq!(lines, vec!["  keep".to_string(), "+ added".to_string()]);
    }

    /// The spans tile the prompt, so the stretch one of them stands for is exactly the
    /// bytes between what came before it and its own length.
    #[test]
    fn a_span_is_cut_back_out_of_the_prompt_it_was_joined_into() {
        let prompt = "be brief\n\nproject says hello";
        let spans = vec![
            PrefixSpan {
                source: micro_types::EventSource::SystemPrompt,
                bytes: 8,
                hash: String::new(),
            },
            PrefixSpan {
                source: micro_types::EventSource::ProjectInstructions,
                bytes: 20,
                hash: String::new(),
            },
        ];

        assert_eq!(cut(prompt, &spans, 0), Some("be brief"));
        assert_eq!(cut(prompt, &spans, 1), Some("\n\nproject says hello"));
    }

    #[test]
    fn the_one_span_that_differs_is_the_one_compared() {
        let spans = |instructions_hash: &str| {
            vec![
                PrefixSpan {
                    source: micro_types::EventSource::SystemPrompt,
                    bytes: 8,
                    hash: "aa".into(),
                },
                PrefixSpan {
                    source: micro_types::EventSource::ProjectInstructions,
                    bytes: 5,
                    hash: instructions_hash.into(),
                },
            ]
        };

        let (source, had, has) = changed_span(
            "be brief\n\nrun",
            &spans("bb"),
            "be brief\n\nfly",
            &spans("cc"),
        );

        assert_eq!(source.as_deref(), Some("project_instructions"));
        assert_eq!(had, "\n\nrun");
        assert_eq!(has, "\n\nfly");
    }

    /// A prompt something replaced outright is not built from the parts the last one was,
    /// so there is no one span to point at and the whole thing is compared.
    #[test]
    fn a_prompt_rebuilt_from_different_parts_is_compared_whole() {
        let before = vec![PrefixSpan {
            source: micro_types::EventSource::SystemPrompt,
            bytes: 8,
            hash: "aa".into(),
        }];
        let after = vec![PrefixSpan {
            source: micro_types::EventSource::Extension(String::new()),
            bytes: 5,
            hash: "bb".into(),
        }];

        let (source, had, has) = changed_span("be brief", &before, "hello", &after);

        assert_eq!(source, None);
        assert_eq!(had, "be brief");
        assert_eq!(has, "hello");
    }
}
