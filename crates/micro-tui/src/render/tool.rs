//! One tool result, drawn.
//!
//! The whole result sits in a band of background whose color says how the call went:
//! pending while it runs, then success or error. That band is what marks a tool block, and
//! it does the work a marker glyph and a frame would otherwise be doing, so there is
//! neither.
//!
//! The header is always one line: what ran, what it acted on, and how it went. Everything
//! below it is the body the reader can open, styled by what the row means — a diff line, a
//! search hit, a line of output.

use crate::diff;
use crate::diff::DiffLine;
use crate::render::transcript::band;
use crate::theme::Theme;
use crate::tools;
use crate::tools::Row;
use crate::transcript::ToolEntry;
use crate::wrap::wrap_spans;
use crate::wrap::wrap_spans_hard;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

/// Columns a row is inset by when it is drawn under a heading of its own, as it is inside
/// an approval prompt. Inside the band the header and the body share a column.
const INDENT: usize = 2;
/// Width of the line-number column beside a search hit.
const NUMBER_WIDTH: usize = 5;

pub fn lines(tool: &ToolEntry, focused: bool, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    if tool.has_custom_render() {
        return custom_lines(tool, focused, theme, width);
    }

    let view = tools::view(
        &tool.name,
        &tool.arguments,
        tool.output.as_deref(),
        tool.is_error,
    );

    let mut rows = header(tool, &view, width, theme);
    let (body, hidden) = view.visible(tool.expanded);
    rows.extend(body_lines(&body, &view, theme, width, 0));
    if hidden > 0 {
        rows.push(hidden_line(hidden, tool.expanded, focused, theme));
    }
    for note in &view.notes {
        rows.extend(wrap_spans(
            &[Span::styled(note.clone(), Style::new().fg(theme.warning))],
            width,
            INDENT,
        ));
    }

    band(rows, width, ground(tool, focused, theme))
}

/// A call an extension is drawing itself, through renderCall/renderResult. Its lines are
/// already composed on the other side of the wire; what is left is where they land.
///
/// Most such tools still want micro's own frame — the padded band whose color says how the
/// call went — and only supply what goes inside it. A tool whose `render_shell` asked for
/// `"self"` wants none of that: it means to draw its own frame on the far side, so this
/// leaves the band off entirely and only spaces the result off from whatever came before.
fn custom_lines(
    tool: &ToolEntry,
    focused: bool,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let rows: Vec<Line<'static>> = tool
        .render_lines()
        .into_iter()
        .flat_map(|text| row_lines(&Row::Plain(text), theme, width, 0))
        .collect();

    if tool.self_framed {
        return std::iter::once(Line::default()).chain(rows).collect();
    }

    band(rows, width, ground(tool, focused, theme))
}

/// The color behind the result: what the call is doing, or — when the reader has picked
/// this one out — that it is the one picked.
fn ground(tool: &ToolEntry, focused: bool, theme: &Theme) -> Color {
    if focused {
        return theme.selected_bg;
    }
    match (&tool.output, tool.is_error) {
        (None, _) => theme.tool_pending_bg,
        (Some(_), true) => theme.tool_error_bg,
        (Some(_), false) => theme.tool_success_bg,
    }
}

/// What ran, what it acted on, and how it went, on one line.
fn header(
    tool: &ToolEntry,
    view: &tools::ToolView,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut spans = vec![Span::styled(
        tool.name.clone(),
        Style::new()
            .fg(theme.tool_title)
            .add_modifier(Modifier::BOLD),
    )];

    if !view.subject.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            view.subject.clone(),
            Style::new().fg(theme.accent),
        ));
    }
    let output = Style::new().fg(theme.tool_output);
    match &view.detail {
        Some(detail) => {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(detail.clone(), output));
        }
        None if tool.output.is_none() => spans.push(Span::styled(" …", output)),
        None => {}
    }

    wrap_spans(&spans, width, INDENT)
}

/// Draw a body, painting each run of diff lines as one block.
///
/// A diff is painted whole rather than a row at a time because a line that replaced exactly
/// one other is marked word by word, and that comparison needs both sides in hand.
fn body_lines(
    rows: &[Row],
    view: &tools::ToolView,
    theme: &Theme,
    width: usize,
    indent: usize,
) -> Vec<Line<'static>> {
    let number_width = match &view.body {
        tools::Body::Diff { number_width, .. } => *number_width,
        _ => 0,
    };

    let mut out = Vec::new();
    let mut index = 0;
    while index < rows.len() {
        let Row::Diff(_) = &rows[index] else {
            out.extend(row_lines(&rows[index], theme, width, indent));
            index += 1;
            continue;
        };

        let end = rows[index..]
            .iter()
            .position(|row| !matches!(row, Row::Diff(_)))
            .map_or(rows.len(), |offset| index + offset);
        let block: Vec<DiffLine> = rows[index..end]
            .iter()
            .filter_map(|row| match row {
                Row::Diff(line) => Some(line.clone()),
                _ => None,
            })
            .collect();

        for mut spans in diff::paint(&block, number_width, theme) {
            if indent > 0 {
                spans.insert(0, Span::raw(" ".repeat(indent)));
            }
            out.extend(wrap_spans_hard(&spans, width, indent + number_width + 2));
        }
        index = end;
    }
    out
}

fn row_lines(row: &Row, theme: &Theme, width: usize, indent: usize) -> Vec<Line<'static>> {
    let pad = " ".repeat(indent);
    // A row that carries code or output is broken at the edge, not re-flowed as prose, and
    // a row it wraps onto is stepped in past the marker so the marker column stays readable.
    let hard = |spans: Vec<Span<'static>>| wrap_spans_hard(&spans, width, indent + 1);

    match row {
        Row::Plain(text) => hard(vec![
            Span::raw(pad),
            Span::styled(text.clone(), Style::new().fg(theme.tool_output)),
        ]),
        // Painted as part of a block, never on its own.
        Row::Diff(_) => Vec::new(),
        Row::Path { path, count } => {
            let mut spans = vec![
                Span::raw(pad),
                Span::styled(path.clone(), Style::new().fg(theme.accent)),
            ];
            if let Some(count) = count {
                spans.push(Span::styled(
                    format!("  {count}"),
                    Style::new().fg(theme.tool_output),
                ));
            }
            wrap_spans(&spans, width, indent + 2)
        }
        Row::Match { line, text } => hard(vec![
            Span::styled(
                format!("{pad}{line:>NUMBER_WIDTH$}  "),
                Style::new().fg(theme.dim),
            ),
            Span::styled(
                text.trim_end().to_string(),
                Style::new().fg(theme.tool_output),
            ),
        ]),
    }
}

/// The affordance under a collapsed result: how much is hidden, and how to see it.
fn hidden_line(hidden: usize, expanded: bool, focused: bool, theme: &Theme) -> Line<'static> {
    let noun = if hidden == 1 { "line" } else { "lines" };
    let mut spans = vec![Span::styled(
        format!("… +{hidden} {noun}"),
        Style::new().fg(theme.tool_output),
    )];
    // The key only helps on the result it would act on, so it is offered only there.
    if focused && !expanded {
        spans.push(Span::styled("  ctrl+o", Style::new().fg(theme.dim)));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrap::text_width;
    use serde_json::json;

    fn entry(name: &str, arguments: serde_json::Value, output: Option<&str>) -> ToolEntry {
        ToolEntry {
            id: "call_1".into(),
            name: name.into(),
            arguments,
            output: output.map(str::to_string),
            is_error: false,
            expanded: false,
            ..Default::default()
        }
    }

    /// The drawn rows without the blank row the band puts above and below them.
    fn rendered(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    // The band's own padding column is not content; dropping exactly it
                    // keeps every assertion about relative indentation honest.
                    .strip_prefix(' ')
                    .unwrap_or_default()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .split_first()
            .map(|(_, rest)| rest.to_vec())
            .unwrap_or_default()
            .split_last()
            .map(|(_, rest)| rest.to_vec())
            .unwrap_or_default()
    }

    fn backgrounds(lines: &[Line<'static>]) -> Vec<Option<Color>> {
        lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.style.bg))
            .collect()
    }

    /// A tool with a registered component draws exactly what it answered rather than the
    /// built-in view — the call and the result lines stacked in that order, the way pi's
    /// own `ToolExecutionComponent` stacks its two renderers.
    #[test]
    fn a_tool_with_a_registered_component_draws_what_it_answered_instead_of_the_built_in_view() {
        let theme = Theme::dark();
        let mut tool = entry("weather", json!({ "city": "lima" }), Some("ignored"));
        tool.call_component_id = Some("component-0".into());
        tool.call_lines = Some(vec!["lima: sunny".into()]);
        tool.result_component_id = Some("component-1".into());
        tool.result_lines = Some(vec!["18°C".into()]);

        assert_eq!(
            rendered(&lines(&tool, false, &theme, 60)),
            vec!["lima: sunny", "18°C"]
        );
    }

    /// `render_shell: "self"` skips micro's own band entirely: no background tint marking
    /// how the call went, no padding column, just the extension's own lines behind one blank
    /// row that keeps them off whatever came before — no trailing blank row, since nothing
    /// here is closing a frame the extension is still drawing.
    #[test]
    fn a_self_framed_tool_draws_without_the_band() {
        let theme = Theme::dark();
        let mut tool = entry("weather", json!({ "city": "lima" }), Some("ignored"));
        tool.call_component_id = Some("component-0".into());
        tool.call_lines = Some(vec!["lima: sunny".into()]);
        tool.self_framed = true;

        let out = lines(&tool, false, &theme, 60);
        let text: Vec<String> = out
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        assert_eq!(text, vec!["", "lima: sunny"]);
        assert!(
            out.iter()
                .flat_map(|line| line.spans.iter())
                .all(|span| span.style.bg.is_none()),
            "no band color behind a self-framed call"
        );
    }

    /// A component answering for only one of the two renderers still draws — the built-in
    /// view is not a fallback for the half nothing registered.
    #[test]
    fn only_a_call_component_still_draws_on_its_own() {
        let theme = Theme::dark();
        let mut tool = entry("weather", json!({ "city": "lima" }), None);
        tool.call_component_id = Some("component-0".into());
        tool.call_lines = Some(vec!["lima: checking...".into()]);

        assert_eq!(
            rendered(&lines(&tool, false, &theme, 60)),
            vec!["lima: checking..."]
        );
    }

    #[test]
    fn a_running_tool_is_one_line_in_a_pending_band() {
        let theme = Theme::dark();
        let tool = entry("read", json!({ "path": "a.rs" }), None);
        let out = lines(&tool, false, &theme, 60);

        assert_eq!(rendered(&out), vec!["read a.rs …"]);
        assert_eq!(out.len(), 3, "a blank row above and below");
        assert!(backgrounds(&out)
            .iter()
            .all(|bg| *bg == Some(theme.tool_pending_bg)));
    }

    #[test]
    fn the_band_says_how_the_call_went() {
        let theme = Theme::dark();
        let mut tool = entry("bash", json!({ "command": "true" }), Some("done"));
        assert_eq!(
            backgrounds(&lines(&tool, false, &theme, 60))[0],
            Some(theme.tool_success_bg)
        );

        tool.is_error = true;
        assert_eq!(
            backgrounds(&lines(&tool, false, &theme, 60))[0],
            Some(theme.tool_error_bg)
        );

        tool.output = None;
        assert_eq!(
            backgrounds(&lines(&tool, false, &theme, 60))[0],
            Some(theme.tool_pending_bg)
        );
    }

    #[test]
    fn the_picked_result_takes_the_selected_ground() {
        let theme = Theme::dark();
        let tool = entry("ls", json!({ "path": "." }), Some("a\nb"));
        assert!(backgrounds(&lines(&tool, true, &theme, 40))
            .iter()
            .all(|bg| *bg == Some(theme.selected_bg)));
    }

    /// A band spans the content it was given plus the column of ground either side of it,
    /// so it reaches both edges of the screen rather than floating inside them.
    #[test]
    fn every_row_of_the_band_reaches_both_edges() {
        let theme = Theme::dark();
        let mut tool = entry("bash", json!({ "command": "cat" }), Some(&"x".repeat(200)));
        tool.expanded = true;
        let banded = 40 + crate::render::transcript::PADDING * 2;
        for line in lines(&tool, false, &theme, 40) {
            let width: usize = line.spans.iter().map(|s| text_width(&s.content)).sum();
            assert_eq!(width, banded);
        }
    }

    #[test]
    fn the_header_names_the_tool_and_what_it_acted_on() {
        let theme = Theme::dark();
        let tool = entry("read", json!({ "path": "a.rs" }), Some("one"));
        let out = lines(&tool, false, &theme, 60);
        // The first span is the band's padding column; the header begins after it.
        let header: Vec<_> = out[1].spans.iter().skip(1).collect();

        assert_eq!(header[0].content.as_ref(), "read");
        assert_eq!(header[0].style.fg, Some(theme.tool_title));
        assert!(header[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(header[2].content.as_ref(), "a.rs");
        assert_eq!(header[2].style.fg, Some(theme.accent));
    }

    #[test]
    fn the_body_shares_a_column_with_the_header() {
        let tool = entry(
            "edit",
            json!({ "path": "a.rs", "old_string": "one\ntwo", "new_string": "one\nTWO" }),
            Some("Edited a.rs"),
        );
        assert_eq!(
            rendered(&lines(&tool, false, &Theme::dark(), 60)),
            vec!["edit a.rs  +1 -1", " 1 one", "-2 two", "+2 TWO"]
        );
    }

    #[test]
    fn a_diff_row_takes_the_color_of_the_change_it_reports() {
        let theme = Theme::dark();
        let tool = entry(
            "edit",
            json!({ "path": "a.rs", "old_string": "one\ntwo", "new_string": "one\nTWO" }),
            Some("Edited a.rs"),
        );
        let out = lines(&tool, false, &theme, 60);
        // Span 0 is the band's padding column, so the row's own first span is the next one.
        let colour = |row: usize| out[row].spans[1].style.fg;
        assert_eq!(colour(2), Some(theme.tool_diff_context));
        assert_eq!(colour(3), Some(theme.tool_diff_removed));
        assert_eq!(colour(4), Some(theme.tool_diff_added));
    }

    #[test]
    fn a_collapsed_read_offers_the_expand_key_only_when_focused() {
        let tool = entry(
            "read",
            json!({ "path": "a.rs" }),
            Some("     1\tone\n     2\ttwo"),
        );

        let unfocused = rendered(&lines(&tool, false, &Theme::dark(), 60));
        assert_eq!(unfocused, vec!["read a.rs  2 lines", "… +2 lines"]);

        let focused = rendered(&lines(&tool, true, &Theme::dark(), 60));
        assert_eq!(focused[1], "… +2 lines  ctrl+o");
    }

    #[test]
    fn expanding_replaces_the_affordance_with_the_output() {
        let mut tool = entry(
            "read",
            json!({ "path": "a.rs" }),
            Some("     1\tone\n     2\ttwo"),
        );
        tool.expanded = true;
        let out = rendered(&lines(&tool, false, &Theme::dark(), 60));
        assert_eq!(out.len(), 3);
        assert!(out[1].contains("one"));
        assert!(!out.iter().any(|line| line.contains("+2 lines")));
    }

    #[test]
    fn a_failed_command_shows_why_it_failed() {
        let theme = Theme::dark();
        let mut tool = entry(
            "bash",
            json!({ "command": "cargo test" }),
            Some("exit code 101\npanicked at src/main.rs"),
        );
        tool.is_error = true;
        let out = lines(&tool, false, &theme, 60);

        assert_eq!(
            rendered(&out),
            vec!["bash cargo test  exit 101", "panicked at src/main.rs"]
        );
        assert!(backgrounds(&out)
            .iter()
            .all(|bg| *bg == Some(theme.tool_error_bg)));
    }

    #[test]
    fn a_search_lists_its_files_and_opens_to_the_matches() {
        let mut tool = entry(
            "grep",
            json!({ "pattern": "fn " }),
            Some("a.rs:1:fn one()\na.rs:9:fn two()"),
        );
        assert_eq!(
            rendered(&lines(&tool, false, &Theme::dark(), 60)),
            vec!["grep fn  2 matches in 1 file", "a.rs  2"]
        );

        tool.expanded = true;
        let out = rendered(&lines(&tool, false, &Theme::dark(), 60));
        assert_eq!(out[2], "    1  fn one()");
        assert_eq!(out[3], "    9  fn two()");
    }

    #[test]
    fn long_output_never_exceeds_the_width() {
        let mut tool = entry("bash", json!({ "command": "cat" }), Some(&"x".repeat(500)));
        tool.expanded = true;
        for width in 8..40 {
            for line in lines(&tool, true, &Theme::dark(), width) {
                let drawn: usize = line
                    .spans
                    .iter()
                    .map(|span| text_width(&span.content))
                    .sum();
                let frame = width + crate::render::transcript::PADDING * 2;
                assert!(drawn <= frame, "row of {drawn} exceeds {frame}");
            }
        }
    }

    /// Painting a diff as a block is what keeps the word-level marks: a line that replaced
    /// exactly one other is compared against it, which needs both sides in one call.
    #[test]
    fn a_one_for_one_replacement_keeps_its_word_marks() {
        let tool = ToolEntry {
            id: "1".into(),
            name: "edit".into(),
            arguments: json!({
                "path": "a.rs",
                "old_string": "    let total = compute(a, b);",
                "new_string": "    let total = compute(a, b, c);",
            }),
            output: Some("Edited a.rs".into()),
            is_error: false,
            expanded: true,
            ..Default::default()
        };

        let marked: Vec<String> = lines(&tool, false, &Theme::dark(), 80)
            .iter()
            .flat_map(|line| line.spans.clone())
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.to_string())
            .collect();

        assert!(!marked.is_empty(), "the changed words are not marked");
        assert!(
            marked.iter().all(|text| !text.starts_with(' ')),
            "indentation should not be lit up: {marked:?}"
        );
        // Only the tail that actually changed: `let total = compute(a,` is common to both
        // sides and stays unlit, as does the indentation ahead of it.
        assert!(
            marked.iter().any(|text| text.contains("c);")),
            "the added argument should be marked: {marked:?}"
        );
        assert!(
            !marked.iter().any(|text| text.contains("compute")),
            "unchanged words should not be lit: {marked:?}"
        );
    }
}
