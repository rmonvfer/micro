//! A `codemode` call, drawn: the script, the calls it made as they ran, and what it output.

use crate::markdown::syntax::Highlighter;
use crate::render::tool::ground;
use crate::render::tool::hidden_line;
use crate::render::transcript::band;
use crate::theme::Theme;
use crate::tools;
use crate::transcript::NestedRow;
use crate::transcript::NestedState;
use crate::transcript::ToolEntry;
use crate::wrap::wrap_spans;
use crate::wrap::wrap_spans_hard;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

/// Wrapped lines of the script shown while collapsed.
const SCRIPT_PREVIEW_LINES: usize = 10;
/// Nested calls shown while collapsed: the most recent ones.
const CALL_PREVIEW_COUNT: usize = 8;
/// Wrapped lines of output shown while collapsed. Output is often one long line of JSON, so the
/// preview counts the lines it wraps to, not the lines it holds.
const OUTPUT_PREVIEW_LINES: usize = 5;
/// Characters of a nested call's arguments shown while collapsed.
const COLLAPSED_ARGUMENT_CHARS: usize = 80;
/// Columns a row is inset by under the header.
const INDENT: usize = 2;

pub fn lines(
    tool: &ToolEntry,
    focused: bool,
    theme: &Theme,
    width: usize,
    pad: usize,
) -> Vec<Line<'static>> {
    let script = tool
        .arguments
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let outcome = tool.output.as_deref().map(Outcome::read);

    let mut rows = header(tool, outcome.as_ref(), theme, width);

    let script = script_lines(script, theme, width);
    rows.extend(limited(
        script,
        tool.expanded,
        SCRIPT_PREVIEW_LINES,
        focused,
        theme,
    ));

    if !tool.nested.is_empty() {
        rows.push(Line::default());
        rows.extend(call_lines(&tool.nested, tool.expanded, theme, width));
    }

    if let Some(outcome) = &outcome {
        if !outcome.output.trim().is_empty() {
            let color = match tool.is_error {
                true => theme.error,
                false => theme.tool_output,
            };
            let output: Vec<Line<'static>> = outcome
                .output
                .trim()
                .lines()
                .flat_map(|line| {
                    wrap_spans_hard(
                        &[Span::styled(
                            line.replace('\t', "    "),
                            Style::new().fg(color),
                        )],
                        width,
                        0,
                    )
                })
                .collect();
            rows.push(Line::default());
            rows.extend(limited(
                output,
                tool.expanded,
                OUTPUT_PREVIEW_LINES,
                focused,
                theme,
            ));
            // The collapsed preview hides the note at the end that names the file.
            if let Some(path) = outcome.full_output_path.as_ref().filter(|_| !tool.expanded) {
                rows.extend(wrap_spans(
                    &[Span::styled(
                        format!("Full output: {path}"),
                        Style::new().fg(theme.muted),
                    )],
                    width,
                    INDENT,
                ));
            }
        }
    }

    band(rows, width, pad, ground(tool, focused, theme))
}

/// What a finished script said, read back out of its result.
struct Outcome {
    /// How long it ran, as the result says.
    wall_time: Option<String>,
    failed: bool,
    /// Everything after the header.
    output: String,
    full_output_path: Option<String>,
}

impl Outcome {
    fn read(text: &str) -> Outcome {
        let mut lines = text.lines();
        let first = lines.clone().next().unwrap_or_default();
        let failed = first == "Script failed";
        if !(failed || first == "Script completed") {
            // Rejected before it ran, such as for bad options: there is no header.
            return Outcome {
                wall_time: None,
                failed: true,
                output: text.to_string(),
                full_output_path: None,
            };
        }
        lines.next();
        let wall_time = lines
            .next()
            .and_then(|line| line.strip_prefix("Wall time "))
            .and_then(|rest| rest.strip_suffix(" seconds"))
            .map(|seconds| format!("{seconds}s"));
        lines.next();
        let output = lines.collect::<Vec<_>>().join("\n");
        let full_output_path = output
            .split("[Full output: ")
            .nth(1)
            .and_then(|rest| rest.split(" (read with offset/limit)]").next())
            .map(str::to_string);
        Outcome {
            wall_time,
            failed,
            output,
            full_output_path,
        }
    }
}

fn header(
    tool: &ToolEntry,
    outcome: Option<&Outcome>,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut spans = vec![Span::styled(
        tools::title(&tool.name),
        Style::new()
            .fg(theme.tool_title)
            .add_modifier(Modifier::BOLD),
    )];
    let output = Style::new().fg(theme.tool_output);
    match outcome {
        None => spans.push(Span::styled(" …", output)),
        Some(outcome) => {
            let mut detail = Vec::new();
            if outcome.failed {
                detail.push("failed".to_string());
            }
            detail.extend(outcome.wall_time.clone());
            if !detail.is_empty() {
                spans.push(Span::styled(format!("  {}", detail.join(", ")), output));
            }
        }
    }
    wrap_spans(&spans, width, INDENT)
}

/// The script, highlighted as JavaScript, the `// @options:` line included.
fn script_lines(script: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let script = script.replace('\r', "").replace('\t', "    ");
    let mut highlighter = Highlighter::new("javascript");
    script
        .trim_end()
        .lines()
        .flat_map(|line| {
            let spans: Vec<Span<'static>> = match highlighter.as_mut() {
                Some(highlighter) => highlighter
                    .line(line)
                    .into_iter()
                    .map(|token| {
                        let style = token
                            .scope
                            .map(|scope| scope.style(theme))
                            .unwrap_or_else(|| Style::new().fg(theme.text));
                        Span::styled(token.text, style)
                    })
                    .collect(),
                None => vec![Span::styled(line.to_string(), Style::new().fg(theme.text))],
            };
            wrap_spans_hard(&spans, width, 0)
        })
        .collect()
}

/// `lines` as they are when expanded, and their first `preview` with a note of the rest when
/// collapsed.
fn limited(
    mut lines: Vec<Line<'static>>,
    expanded: bool,
    preview: usize,
    focused: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if expanded || lines.len() <= preview {
        return lines;
    }
    let hidden = lines.len() - preview;
    lines.truncate(preview);
    lines.push(hidden_line(hidden, expanded, focused, theme));
    lines
}

/// One line per nested call, with how it went. Collapsed, only the latest calls show, and a
/// failed call's error shows only when expanded.
fn call_lines(
    calls: &[NestedRow],
    expanded: bool,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let shown = match expanded {
        true => calls,
        false => &calls[calls.len().saturating_sub(CALL_PREVIEW_COUNT)..],
    };
    let mut lines = Vec::new();
    let earlier = calls.len() - shown.len();
    if earlier > 0 {
        let noun = if earlier == 1 { "call" } else { "calls" };
        lines.push(Line::from(Span::styled(
            format!("… {earlier} earlier {noun}"),
            Style::new().fg(theme.muted),
        )));
    }
    for call in shown {
        let (icon, color) = match call.state {
            NestedState::Running => ("…", theme.warning),
            NestedState::Ok => ("✓", theme.success),
            NestedState::Error => ("✗", theme.error),
            NestedState::Cancelled => ("⊘", theme.muted),
        };
        let arguments = match !expanded && call.arguments.chars().count() > COLLAPSED_ARGUMENT_CHARS
        {
            true => format!(
                "{}...",
                call.arguments
                    .chars()
                    .take(COLLAPSED_ARGUMENT_CHARS - 3)
                    .collect::<String>()
            ),
            false => call.arguments.clone(),
        };
        let mut spans = vec![
            Span::styled(icon, Style::new().fg(color)),
            Span::raw(" "),
            Span::styled(tools::title(&call.name), Style::new().fg(theme.tool_title)),
        ];
        if !arguments.is_empty() && arguments != "{}" {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(arguments, Style::new().fg(theme.muted)));
        }
        if let Some(duration) = call.duration_ms {
            spans.push(Span::styled(
                format!(" {}", duration_text(duration)),
                Style::new().fg(theme.dim),
            ));
        }
        lines.extend(wrap_spans(&spans, width, INDENT));
        if let Some(error) = call.error.as_ref().filter(|_| expanded) {
            for line in error.lines() {
                lines.extend(wrap_spans_hard(
                    &[
                        Span::raw("    "),
                        Span::styled(line.to_string(), Style::new().fg(theme.error)),
                    ],
                    width,
                    4,
                ));
            }
        }
    }
    lines
}

fn duration_text(milliseconds: u64) -> String {
    match milliseconds < 1000 {
        true => format!("{milliseconds}ms"),
        false => format!("{:.1}s", milliseconds as f64 / 1000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .collect()
    }

    fn codemode(code: &str, output: Option<&str>) -> ToolEntry {
        ToolEntry {
            id: "call_1".into(),
            name: "codemode".into(),
            arguments: json!({ "code": code }),
            output: output.map(str::to_string),
            ..Default::default()
        }
    }

    fn row(name: &str, state: NestedState) -> NestedRow {
        NestedRow {
            id: format!("call_1/{name}"),
            name: name.into(),
            arguments: json!({ "path": name }).to_string(),
            state,
            duration_ms: Some(1500),
            error: Some("it broke".into()),
            started: None,
        }
    }

    #[test]
    fn a_running_script_shows_its_code_and_its_calls_so_far() {
        let mut tool = codemode(
            "const a = await tools.read({ path: 'x' });\nreturn a;",
            None,
        );
        tool.nested = vec![
            row("read", NestedState::Ok),
            row("mcp__docs__search", NestedState::Running),
        ];
        let drawn = text(&lines(&tool, false, &Theme::dark(), 80, 0));
        assert!(drawn.contains(&"codemode …".to_string()), "{drawn:?}");
        assert!(drawn.contains(&"const a = await tools.read({ path: 'x' });".to_string()));
        assert!(
            drawn.contains(&"✓ read {\"path\":\"read\"} 1.5s".to_string()),
            "{drawn:?}"
        );
        assert!(
            drawn.iter().any(|line| line.starts_with("… docs/search")),
            "{drawn:?}"
        );
        assert!(
            !drawn.iter().any(|line| line.contains("it broke")),
            "errors wait for expansion"
        );
    }

    #[test]
    fn the_output_drops_the_header_and_is_cut_by_wrapped_lines() {
        let long = "x".repeat(400);
        let tool = codemode(
            "return 1;",
            Some(&format!(
                "Script completed\nWall time 0.3 seconds\nOutput:\n\n{long}"
            )),
        );
        let drawn = text(&lines(&tool, false, &Theme::dark(), 40, 0));
        assert!(
            drawn.iter().any(|line| line == "codemode  0.3s"),
            "{drawn:?}"
        );
        assert!(!drawn.iter().any(|line| line.contains("Script completed")));
        let output_rows = drawn.iter().filter(|line| line.starts_with("xxxx")).count();
        assert_eq!(output_rows, OUTPUT_PREVIEW_LINES, "{drawn:?}");
        assert!(
            drawn.iter().any(|line| line.starts_with("… +")),
            "{drawn:?}"
        );
    }

    #[test]
    fn a_failed_script_says_so_and_names_the_full_output() {
        let tool = ToolEntry {
            is_error: true,
            ..codemode(
                "throw 1",
                Some("Script failed\nWall time 1.0 seconds\nOutput:\n\nWarning: truncated\n[Full output: /tmp/out.txt (read with offset/limit)]"),
            )
        };
        let drawn = text(&lines(&tool, false, &Theme::dark(), 80, 0));
        assert!(
            drawn.iter().any(|line| line == "codemode  failed, 1.0s"),
            "{drawn:?}"
        );
        assert!(
            drawn.contains(&"Full output: /tmp/out.txt".to_string()),
            "{drawn:?}"
        );
    }

    #[test]
    fn collapsed_calls_show_the_latest_and_count_the_rest() {
        let mut tool = codemode(
            "x",
            Some("Script completed\nWall time 0.1 seconds\nOutput:\n"),
        );
        tool.nested = (0..12)
            .map(|n| row(&format!("t{n}"), NestedState::Error))
            .collect();
        let drawn = text(&lines(&tool, false, &Theme::dark(), 80, 0));
        assert!(
            drawn.contains(&"… 4 earlier calls".to_string()),
            "{drawn:?}"
        );
        assert!(drawn.iter().any(|line| line.starts_with("✗ t11")));
        assert!(!drawn.iter().any(|line| line.starts_with("✗ t3 ")));

        tool.expanded = true;
        let drawn = text(&lines(&tool, false, &Theme::dark(), 80, 0));
        assert!(drawn.iter().any(|line| line.starts_with("✗ t0")));
        assert!(drawn.contains(&"it broke".to_string()), "{drawn:?}");
    }
}
