//! The two overlays a command can open: a list to choose from, and a prompt for a key.
//!
//! Both are shaped after ohm's selectors — a title, a filter you type into, an arrow on the
//! highlighted row, a marker on what is already in use, and a count when the list scrolls.

use crate::app::KeyPrompt;
use crate::picker::Picker;
use crate::picker::MAX_VISIBLE;
use crate::render::tint;
use crate::theme::Theme;
use crate::wrap::text_width;
use crate::wrap::truncate;
use crate::wrap::wrap_spans;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

/// Columns the selection arrow occupies. It is three bytes wide but two columns.
const MARKER_WIDTH: usize = 2;
/// Gap between an item's label and its detail.
const COLUMN_GAP: usize = 2;

pub fn picker_lines(
    picker: &Picker,
    theme: &Theme,
    width: usize,
    max_rows: usize,
) -> Vec<Line<'static>> {
    let mut head = vec![title(picker, theme), filter(picker, theme)];
    let tail = vec![hint("↑↓ move · enter choose · esc cancel", theme)];

    let budget = max_rows.saturating_sub(head.len() + tail.len()).max(1);
    let window = picker.window(MAX_VISIBLE.min(budget.saturating_sub(1).max(1)));

    if picker.is_empty() {
        head.push(Line::from(vec![Span::styled(
            "  nothing matches".to_string(),
            theme.secondary(),
        )]));
    } else {
        let matches = picker.matches();
        let labels = window
            .clone()
            .filter_map(|index| matches.get(index))
            .map(|item| text_width(&item.label))
            .max()
            .unwrap_or(0);

        for index in window.clone() {
            let Some(item) = matches.get(index) else {
                continue;
            };
            head.push(row(
                &item.label,
                &item.detail,
                item.current,
                index == picker.selected(),
                labels,
                theme,
                width,
            ));
        }
        if window.len() < matches.len() {
            head.push(Line::from(vec![Span::styled(
                format!("  ({}/{})", picker.selected() + 1, matches.len()),
                theme.secondary(),
            )]));
        }
    }

    head.extend(tail);
    head.truncate(max_rows.max(1));
    head.into_iter()
        .map(|line| tint(line, width, theme.surface))
        .collect()
}

fn title(picker: &Picker, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            picker.title().to_string(),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   {} to choose from", picker.total()),
            theme.dimmed(),
        ),
    ])
}

/// The query, prompted with `> ` and carrying a cursor.
///
/// ohm marks the cursor by reversing the character under it rather than by drawing a block
/// beside it, so at the end of the line the reversed cell is a space.
fn filter(picker: &Picker, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled("  > ", Style::new().fg(theme.accent)),
        Span::styled(picker.query().to_string(), Style::new().fg(theme.text)),
        Span::styled(" ", Style::new().fg(theme.text).add_modifier(Modifier::REVERSED)),
    ])
}

fn hint(text: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![Span::styled(format!("  {text}"), theme.dimmed())])
}

fn row(
    label: &str,
    detail: &str,
    current: bool,
    selected: bool,
    column: usize,
    theme: &Theme,
    width: usize,
) -> Line<'static> {
    let marker = match selected {
        true => "→ ",
        false => "  ",
    };
    let style = match selected {
        true => Style::new().fg(theme.accent),
        false => Style::new().fg(theme.text),
    };

    // The tick marking what is in use keeps its two columns whether or not it is drawn, so
    // the rows below it do not shift as the selection moves.
    let available = width.saturating_sub(MARKER_WIDTH + 2);
    let column = column.min(available.saturating_sub(1).max(1));
    let shown = truncate(label, column);

    let mut spans = vec![
        Span::styled(marker, Style::new().fg(theme.accent)),
        Span::styled(shown.clone(), style),
    ];

    let used = MARKER_WIDTH + column + COLUMN_GAP;
    let remaining = width.saturating_sub(used + 2);
    if !detail.is_empty() && remaining > 0 {
        let padding = column - text_width(&shown) + COLUMN_GAP;
        spans.push(Span::raw(" ".repeat(padding)));
        spans.push(Span::styled(truncate(detail, remaining), theme.secondary()));
    }
    if current {
        spans.push(Span::styled(" ✓", Style::new().fg(theme.success)));
    }
    Line::from(spans)
}

/// The prompt for a credential. What is typed is never drawn back.
pub fn key_prompt_lines(prompt: &KeyPrompt, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("sign in to {}", prompt.provider),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
    ])];

    if !prompt.env_names.is_empty() {
        out.extend(wrap_spans(
            &[
                Span::raw("  "),
                Span::styled(
                    format!(
                        "paste an API key, or leave this and set {}",
                        prompt.env_names.join(" or ")
                    ),
                    theme.dimmed(),
                ),
            ],
            width,
            2,
        ));
    }

    out.push(Line::default());
    out.push(Line::from(vec![
        Span::styled("  > ", Style::new().fg(theme.accent)),
        Span::styled(
            "•".repeat(prompt.len().min(width.saturating_sub(6))),
            Style::new().fg(theme.text),
        ),
        Span::styled(
            " ",
            Style::new().fg(theme.text).add_modifier(Modifier::REVERSED),
        ),
    ]));
    out.push(hint("enter save · esc cancel", theme));

    out.into_iter()
        .map(|line| tint(line, width, theme.surface))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_commands::Picker as Choices;
    use micro_commands::PickerItem;

    fn picker() -> Picker {
        Picker::new(Choices::new(
            "Select a model",
            vec![
                PickerItem::new("anthropic/claude-opus-5", "200k context", "/model opus"),
                PickerItem::new("anthropic/claude-sonnet-5", "200k context", "/model sonnet")
                    .current(true),
                PickerItem::new("google/gemini-2.5-pro", "1M context", "/model gemini"),
            ],
        ))
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
    fn a_picker_names_itself_and_marks_what_is_in_use() {
        let out = rendered(&picker_lines(&picker(), &Theme::dark(), 70, 20));
        assert!(out[0].starts_with("  Select a model"));
        assert!(out[0].contains("3 to choose from"));

        let current = out
            .iter()
            .find(|line| line.contains("sonnet"))
            .expect("the current model");
        assert!(current.ends_with('✓'));
        assert!(current.starts_with("→ "), "it opens on what is in use");
    }

    #[test]
    fn the_filter_shows_what_has_been_typed() {
        let mut picker = picker();
        picker.push("gem");
        let out = rendered(&picker_lines(&picker, &Theme::dark(), 70, 20));
        assert_eq!(out[1], "  > gem");
        // The cursor is a reversed cell rather than a glyph, so at the end of the line it is
        // a reversed space — invisible in the text, which is why the style is what is checked.
        let line = filter(&picker, &Theme::dark());
        let cursor = line.spans.last().expect("a cursor cell");
        assert_eq!(cursor.content, " ");
        assert!(cursor.style.add_modifier.contains(Modifier::REVERSED));
        assert!(out.iter().any(|line| line.contains("gemini")));
        assert!(!out.iter().any(|line| line.contains("opus")));
    }

    #[test]
    fn an_empty_result_says_so_and_keeps_the_keys_on_screen() {
        let mut picker = picker();
        picker.push("zzz");
        let out = rendered(&picker_lines(&picker, &Theme::dark(), 70, 20));
        assert!(out.iter().any(|line| line.contains("nothing matches")));
        assert!(out.last().unwrap().contains("esc cancel"));
    }

    #[test]
    fn a_long_list_reports_where_you_are() {
        let items = (0..40)
            .map(|index| PickerItem::new(format!("item-{index}"), "", format!("/pick {index}")))
            .collect();
        let mut picker = Picker::new(Choices::new("Pick", items));
        for _ in 0..5 {
            picker.select_next();
        }
        let out = rendered(&picker_lines(&picker, &Theme::dark(), 70, 20));
        assert!(out.iter().any(|line| line.contains("(6/40)")), "{out:?}");
    }

    #[test]
    fn a_picker_never_outgrows_its_budget_or_its_width() {
        let items = (0..40)
            .map(|index| {
                PickerItem::new(
                    format!("a-very-long-model-identifier-{index}"),
                    "some detail that runs on",
                    "/pick",
                )
            })
            .collect();
        let picker = Picker::new(Choices::new("Pick", items));

        for width in 12..80 {
            for rows in 4..20 {
                let out = picker_lines(&picker, &Theme::dark(), width, rows);
                assert!(out.len() <= rows, "{} rows exceed {rows}", out.len());
                for line in out {
                    let drawn: usize = line.spans.iter().map(|s| text_width(&s.content)).sum();
                    assert_eq!(drawn, width);
                }
            }
        }
    }

    #[test]
    fn a_key_prompt_masks_what_is_typed_and_names_the_variables() {
        let prompt = KeyPrompt::for_test("anthropic", vec!["ANTHROPIC_API_KEY".into()], "secret");
        let out = rendered(&key_prompt_lines(&prompt, &Theme::dark(), 70));

        assert!(out[0].contains("sign in to anthropic"));
        assert!(out[1].contains("ANTHROPIC_API_KEY"));
        assert!(out.iter().any(|line| line.contains("••••••")));
        assert!(
            !out.iter().any(|line| line.contains("secret")),
            "the key is never drawn back"
        );
    }
}
