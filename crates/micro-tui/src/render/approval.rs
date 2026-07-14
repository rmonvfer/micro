//! The approval prompt.
//!
//! It has one job: let the user see exactly what they are agreeing to without going looking
//! for it. The call is shown in full — a shell command line by line, a file change as the
//! diff it would apply — and the option that grants more than this one call says in words
//! what it would remember.

use crate::approval::Choice;
use crate::render::tint;
use crate::render::tool;
use crate::theme::Theme;
use crate::tools;
use crate::wrap::wrap_spans;
use crate::wrap::wrap_spans_hard;
use micro_policy::ApprovalRequest;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

/// Draw the prompt, in no more than `max_rows` rows.
pub fn lines(
    request: &ApprovalRequest,
    selected: Choice,
    waiting: usize,
    theme: &Theme,
    width: usize,
    max_rows: usize,
) -> Vec<Line<'static>> {
    let view = tools::preview(&request.tool, &request.arguments);
    let heading = title(request, &view, waiting, theme);

    let mut head = vec![heading.clone()];
    head.extend(wrap_spans(
        &[
            Span::raw("  "),
            Span::styled(request.reason.clone(), theme.dimmed()),
        ],
        width,
        2,
    ));
    head.push(Line::default());

    let mut tail = vec![Line::default()];
    tail.extend(Choice::ALL.map(|choice| option(choice, choice == selected, request, theme)));

    let budget = max_rows.saturating_sub(head.len() + tail.len());
    let mut out = head;
    out.extend(subject(request, &view, theme, width, budget));
    out.extend(tail.iter().cloned());

    // Squeezed into fewer rows than the furniture alone needs, the answers and what they
    // answer are the parts that cannot go.
    if out.len() > max_rows {
        out = std::iter::once(heading)
            .chain(tail)
            .take(max_rows)
            .collect();
    }

    out.into_iter()
        .map(|line| tint(line, width, theme.surface))
        .collect()
}

fn title(
    request: &ApprovalRequest,
    view: &tools::ToolView,
    waiting: usize,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(
            "approve".to_string(),
            Style::new().fg(theme.warning).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            request.tool.clone(),
            Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
    ];

    // A file change fills the body with its diff, so the path and the size of the change go
    // here, where a command would otherwise have been named.
    if !view.body.is_empty() {
        if !view.subject.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(view.subject.clone(), theme.secondary()));
        }
        if let Some(detail) = &view.detail {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(detail.clone(), theme.dimmed()));
        }
    }

    if waiting > 0 {
        let noun = if waiting == 1 { "call" } else { "calls" };
        spans.push(Span::styled(
            format!("   {waiting} more {noun} waiting"),
            theme.dimmed(),
        ));
    }
    Line::from(spans)
}

/// The call itself: a diff for a file change, the command for anything else.
fn subject(
    request: &ApprovalRequest,
    view: &tools::ToolView,
    theme: &Theme,
    width: usize,
    budget: usize,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    if !view.body.is_empty() {
        let (rows, _) = view.visible(true);
        out.extend(tool::render_body(&rows, view, theme, width));
        return capped(out, budget, theme);
    }

    // Everything else is described by its subject. A shell command keeps its own line
    // breaks, because a command that spans lines is read one line at a time.
    let Some(text) = &request.subject else {
        return out;
    };
    for line in text.split('\n') {
        out.extend(wrap_spans_hard(
            &[
                Span::raw("  "),
                Span::styled(line.to_string(), Style::new().fg(theme.text)),
            ],
            width,
            4,
        ));
    }
    capped(out, budget, theme)
}

/// Keep the call inside its budget. The answers have to stay on screen, so when something
/// must give way it is the call — and the reader is told, because approving what you cannot
/// see is exactly what this prompt exists to prevent.
fn capped(mut rows: Vec<Line<'static>>, budget: usize, theme: &Theme) -> Vec<Line<'static>> {
    if rows.len() <= budget {
        return rows;
    }
    rows.truncate(budget.saturating_sub(1));
    rows.push(Line::from(vec![Span::styled(
        "  … more than fits here; decline if you cannot see enough to judge".to_string(),
        Style::new().fg(theme.warning),
    )]));
    rows
}

fn option(
    choice: Choice,
    selected: bool,
    request: &ApprovalRequest,
    theme: &Theme,
) -> Line<'static> {
    // ohm marks the selected row of a list with an arrow, never a chevron.
    let marker = if selected { "→ " } else { "  " };
    let label = match selected {
        true => Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
        false => Style::new().fg(theme.muted),
    };

    let mut spans = vec![
        Span::styled(format!("  {marker}"), Style::new().fg(theme.accent)),
        Span::styled(
            format!("{:<6}", choice.key()),
            Style::new().fg(theme.accent),
        ),
        Span::styled(choice.label().to_string(), label),
    ];
    // A session grant is the only answer that reaches past this one call, so it is the only
    // one that has to say what it would let through next time.
    if choice == Choice::Session {
        spans.push(Span::styled(
            format!("  - remembers exactly {}", request.key),
            theme.dimmed(),
        ));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrap::text_width;
    use serde_json::json;

    fn bash(command: &str) -> ApprovalRequest {
        ApprovalRequest {
            tool: "bash".into(),
            subject: Some(command.into()),
            arguments: json!({ "command": command }),
            reason: "policy asks before running a shell command".into(),
            key: format!("bash:{command}"),
        }
    }

    fn rendered(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn the_prompt_names_the_tool_the_reason_and_the_call() {
        let out = rendered(&lines(
            &bash("rm -rf build"),
            Choice::Once,
            0,
            &Theme::dark(),
            60,
            20,
        ));
        assert_eq!(out[0], "  approve bash");
        assert_eq!(out[1], "  policy asks before running a shell command");
        assert!(out.iter().any(|line| line.contains("rm -rf build")));
    }

    #[test]
    fn every_answer_is_offered_with_its_key() {
        let out = rendered(&lines(&bash("ls"), Choice::Once, 0, &Theme::dark(), 80, 20));
        for choice in Choice::ALL {
            assert!(
                out.iter()
                    .any(|line| line.contains(choice.key()) && line.contains(choice.label())),
                "{} is not offered",
                choice.label()
            );
        }
    }

    #[test]
    fn the_session_option_states_what_it_would_remember() {
        let out = rendered(&lines(
            &bash("cargo test"),
            Choice::Once,
            0,
            &Theme::dark(),
            90,
            20,
        ));
        let session = out
            .iter()
            .find(|line| line.contains("allow for this session"))
            .expect("the session option");
        assert!(
            session.contains("remembers exactly bash:cargo test"),
            "{session}"
        );

        let once = out
            .iter()
            .find(|line| line.contains("allow once"))
            .expect("the once option");
        assert!(
            !once.contains("remembers"),
            "only a grant that lasts says so"
        );
    }

    #[test]
    fn the_highlighted_answer_is_marked() {
        let out = rendered(&lines(
            &bash("ls"),
            Choice::Session,
            0,
            &Theme::dark(),
            80,
            20,
        ));
        let marked: Vec<&String> = out.iter().filter(|line| line.contains('→')).collect();
        assert_eq!(marked.len(), 1);
        assert!(marked[0].contains("allow for this session"));
    }

    #[test]
    fn a_queue_behind_the_prompt_is_reported() {
        let out = rendered(&lines(&bash("ls"), Choice::Once, 2, &Theme::dark(), 80, 20));
        assert!(out[0].contains("2 more calls waiting"), "{}", out[0]);

        let one = rendered(&lines(&bash("ls"), Choice::Once, 1, &Theme::dark(), 80, 20));
        assert!(one[0].contains("1 more call waiting"));
    }

    #[test]
    fn a_multi_line_command_keeps_its_lines() {
        let out = rendered(&lines(
            &bash("cd build\nmake clean\nmake all"),
            Choice::Once,
            0,
            &Theme::dark(),
            60,
            20,
        ));
        for line in ["cd build", "make clean", "make all"] {
            assert!(
                out.iter().any(|row| row.trim() == line),
                "{line} is missing"
            );
        }
    }

    #[test]
    fn a_file_change_is_shown_as_its_diff() {
        let request = ApprovalRequest {
            tool: "edit".into(),
            subject: Some("src/main.rs".into()),
            arguments: json!({
                "path": "src/main.rs",
                "old_string": "fn main() {\n    old();\n}",
                "new_string": "fn main() {\n    new();\n}",
            }),
            reason: "policy asks before changing a file".into(),
            key: "edit:src/main.rs".into(),
        };
        let out = rendered(&lines(&request, Choice::Once, 0, &Theme::dark(), 60, 20));
        assert_eq!(
            out[0], "  approve edit src/main.rs  +1 -1",
            "the diff fills the body, so the path is named in the title"
        );
        // The gutter carries the marker and the line the change lands on.
        assert!(
            out.iter().any(|line| line.trim() == "-2     old();"),
            "{out:?}"
        );
        assert!(out.iter().any(|line| line.trim() == "+2     new();"));
        assert!(out.iter().any(|line| line.trim() == "1 fn main() {"));
    }

    #[test]
    fn the_prompt_never_outgrows_its_budget() {
        let command = (0..200)
            .map(|index| format!("echo line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        for budget in 10..30 {
            let out = lines(&bash(&command), Choice::Once, 0, &Theme::dark(), 60, budget);
            assert!(out.len() <= budget, "{} rows exceed {budget}", out.len());
            // Whatever gets cut, the answers stay reachable.
            let text = rendered(&out);
            for choice in Choice::ALL {
                assert!(text.iter().any(|line| line.contains(choice.label())));
            }
        }
    }

    #[test]
    fn a_truncated_call_says_it_is_truncated() {
        let command = (0..50)
            .map(|index| format!("echo {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = rendered(&lines(
            &bash(&command),
            Choice::Once,
            0,
            &Theme::dark(),
            60,
            14,
        ));
        assert!(out
            .iter()
            .any(|line| line.contains("decline if you cannot see enough")));
    }

    #[test]
    fn every_row_is_tinted_across_the_full_width() {
        let out = lines(&bash("ls"), Choice::Once, 0, &Theme::dark(), 40, 20);
        for line in &out {
            let width: usize = line.spans.iter().map(|s| text_width(&s.content)).sum();
            assert_eq!(width, 40);
            assert!(line
                .spans
                .iter()
                .all(|span| span.style.bg == Some(Theme::dark().surface)));
        }
    }
}
