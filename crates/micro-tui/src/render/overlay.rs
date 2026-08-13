//! The two overlays a command can open: a list to choose from, and a prompt for a key.
//!
//! Both are shaped after ohm's selectors — a title, a filter you type into, an arrow on the
//! highlighted row, a marker on what is already in use, and a count when the list scrolls.

use crate::app::KeyPrompt;
use crate::picker::Picker;
use crate::picker::MAX_VISIBLE;
use crate::render::hints;
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
/// Below this width a row has no room for two columns.
const NARROW: usize = 40;
/// A detail shorter than this says nothing, so it is left off.
const MIN_DETAIL: usize = 10;

pub fn picker_lines(
    picker: &Picker,
    theme: &Theme,
    width: usize,
    max_rows: usize,
) -> Vec<Line<'static>> {
    // A selector is framed between two rules and stands where the prompt stands, so the
    // rule below it is the one that was already there and the list grows upward.
    //
    // Two shapes. A settled handful of choices — a palette, a reasoning level — is the
    // list and nothing else: it is read at a glance and anything around it is in the way.
    // A list that is searched, or that was put to the reader rather than opened by them,
    // says what it is first and gives its query line room to be seen.
    let mut head = vec![rule(theme, width)];
    let plain = !picker.searchable() && !picker.titled();

    if !plain {
        head.push(Line::default());
        // What the list is: a warning about what it leaves out where there is one, and
        // otherwise its name.
        match picker.hint() {
            Some(saying) => head.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(saying.to_string(), Style::new().fg(theme.warning)),
            ])),
            None => head.push(title(picker, theme)),
        }
        // Which of the two views is showing, and the key that swaps them.
        if picker.has_scopes() {
            head.push(scopes(picker, theme));
        }
        head.push(Line::default());
        if picker.searchable() {
            head.push(filter(picker, theme));
            head.push(Line::default());
        }
    }

    let mut tail = match plain {
        true => vec![rule(theme, width)],
        false => vec![Line::default(), rule(theme, width)],
    };
    if picker.titled() {
        tail.insert(1, hint("↑↓ navigate · enter select · esc cancel", theme));
    }
    // What is happening behind the list, said under it where a note would go.
    if let Some((text, ok)) = picker.status() {
        let style = match ok {
            true => Style::new().fg(theme.success),
            false => theme.dimmed(),
        };
        tail.insert(0, Line::default());
        tail.insert(1, Line::from(vec![Span::styled(format!("  {text}"), style)]));
    }

    // The chosen row's note, for what the row itself has no room for.
    if let Some(note) = picker.selected_item().and_then(|item| item.note.clone()) {
        tail.insert(0, Line::default());
        tail.insert(
            1,
            Line::from(vec![Span::styled(format!("  {note}"), theme.dimmed())]),
        );
    }

    let budget = max_rows.saturating_sub(head.len() + tail.len()).max(1);
    let window = picker.window(MAX_VISIBLE.min(budget.saturating_sub(1).max(1)));

    if picker.is_empty() {
        head.push(Line::from(vec![Span::styled(
            "  nothing matches".to_string(),
            theme.secondary(),
        )]));
    } else {
        let matches = picker.matches();
        let labels = picker.column();

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

/// What the list is called, for a list the reader did not open themselves.
fn title(picker: &Picker, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            picker.title().to_string(),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
    ])
}

/// The two views a shortlist gives, with the one showing picked out.
fn scopes(picker: &Picker, theme: &Theme) -> Line<'static> {
    let muted = theme.dimmed();
    let chosen = Style::new().fg(theme.accent);
    let style = |scope| match picker.scope() == scope {
        true => chosen,
        false => muted,
    };
    Line::from(vec![
        Span::styled("  Scope: ", muted),
        Span::styled("all", style(crate::picker::Scope::All)),
        Span::styled(" | ", muted),
        Span::styled("scoped", style(crate::picker::Scope::Scoped)),
        Span::styled(
            format!("    {} scope (all/scoped)", hints::key_text("tab")),
            muted,
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

/// A rule the width of the interface, which is what an overlay is framed by.
fn rule(theme: &Theme, width: usize) -> Line<'static> {
    Line::from(vec![Span::styled(
        "─".repeat(width.max(1)),
        Style::new().fg(theme.border),
    )])
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
    // A narrow terminal has no room for two columns, so the detail is dropped rather than
    // squeezed into a few characters that say nothing.
    let column = match width > NARROW {
        true => column,
        false => 0,
    };
    // No column asked for is a row of badges: the label is as long as it is, and the detail
    // follows one space after it rather than lining up with the row above.
    let column = match column {
        0 => 0,
        asked => asked.min(available.saturating_sub(1).max(1)),
    };
    let shown = truncate(label, column.max(available));

    let mut spans = vec![
        Span::styled(marker, Style::new().fg(theme.accent)),
        Span::styled(shown.clone(), style),
    ];

    let padding = match column {
        0 => 1,
        _ => column.saturating_sub(text_width(&shown)) + COLUMN_GAP,
    };
    let used = MARKER_WIDTH + text_width(&shown) + padding;
    let remaining = width.saturating_sub(used + 2);
    // Room for a few characters is not room for a detail: below this the label has the row
    // to itself.
    let room = match column {
        0 => remaining > 0,
        _ => remaining > MIN_DETAIL,
    };
    if !detail.is_empty() && room {
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
    let title = match prompt.masked {
        true => format!("sign in to {}", prompt.provider),
        false => prompt.provider.clone(),
    };
    let mut out = vec![Line::from(vec![
        Span::raw("  "),
        Span::styled(
            title,
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
    ])];

    if !prompt.env_names.is_empty() {
        let note = match prompt.masked {
            true => format!(
                "paste an API key, or leave this and set {}",
                prompt.env_names.join(" or ")
            ),
            false => prompt.env_names.join(" "),
        };
        out.extend(wrap_spans(
            &[Span::raw("  "), Span::styled(note, theme.dimmed())],
            width,
            2,
        ));
    }

    out.push(Line::default());
    out.push(Line::from(vec![
        Span::styled("  > ", Style::new().fg(theme.accent)),
        Span::styled(
            match prompt.masked {
                // A credential is never drawn back, however much of it has been typed.
                true => "•".repeat(prompt.len().min(width.saturating_sub(6))),
                false => crate::wrap::truncate(prompt.text(), width.saturating_sub(6)).to_string(),
            },
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
        Picker::new(
            Choices::new(
                "Select a model",
                vec![
                    PickerItem::new("anthropic/claude-opus-5", "200k context", "/model opus"),
                    PickerItem::new("anthropic/claude-sonnet-5", "200k context", "/model sonnet")
                        .current(true),
                    PickerItem::new("google/gemini-2.5-pro", "1M context", "/model gemini"),
                ],
            )
            .searchable(),
        )
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

    /// A list that is searched says what it is and gives its query line room; the rule
    /// below it is the prompt's own, so the list grows upward off it.
    #[test]
    fn a_searched_list_names_itself_above_its_query() {
        let out = rendered(&picker_lines(&picker(), &Theme::dark(), 70, 20));
        assert!(out[0].starts_with('─'), "framed above: {:?}", out[0]);
        assert!(out.last().unwrap().starts_with('─'), "and below");
        assert_eq!(out[1], "", "room above the name");
        assert_eq!(out[2], "  Select a model");
        assert_eq!(out[4], "  >", "the query line, with room around it");

        let current = out
            .iter()
            .find(|line| line.contains("sonnet"))
            .expect("the current model");
        assert!(current.ends_with('✓'));
        assert!(current.starts_with("→ "), "it opens on what is in use");
    }

    /// A settled handful of choices is the list and nothing else: it is read at a glance,
    /// and a name and a query line above it would only be in the way.
    #[test]
    fn a_short_settled_list_is_only_the_list() {
        let levels = Picker::new(Choices::new(
            "Thinking",
            vec![
                PickerItem::new("off", "no reasoning", "/thinking off").current(true),
                PickerItem::new("low", "a little", "/thinking low"),
            ],
        ));
        let out = rendered(&picker_lines(&levels, &Theme::dark(), 70, 20));
        assert_eq!(out.len(), 4, "two rules and two rows: {out:?}");
        assert!(out[0].starts_with('─'));
        assert!(out[1].starts_with("→ off"));
        assert!(out[3].starts_with('─'));
    }

    /// A workspace's shortlist is what the list opens on, with the whole catalog a key
    /// away, and what is happening behind it is said under it.
    #[test]
    fn a_shortlist_is_what_the_list_opens_on() {
        let all = vec![
            PickerItem::new("claude-opus-5", "[anthropic]", "/model o").current(true),
            PickerItem::new("gemini-3-pro", "[google]", "/model g"),
        ];
        let mut list = Picker::new(
            Choices::new("Select a model", all.clone())
                .scoping(vec![all[0].clone()])
                .searchable()
                .laid_out(micro_commands::PickerLayout::Badges),
        );

        let out = rendered(&picker_lines(&list, &Theme::dark(), 76, 24));
        assert!(
            out.iter().any(|line| line.contains("Scope: all | scoped")),
            "{out:?}"
        );
        assert!(out.iter().any(|line| line.contains("claude-opus-5")));
        assert!(
            !out.iter().any(|line| line.contains("gemini")),
            "it opens on the shortlist: {out:?}"
        );

        list.toggle_scope();
        let out = rendered(&picker_lines(&list, &Theme::dark(), 76, 24));
        assert!(
            out.iter().any(|line| line.contains("gemini")),
            "and the whole catalog is a key away: {out:?}"
        );

        list.set_status("Model catalogs refreshed.", true);
        let out = rendered(&picker_lines(&list, &Theme::dark(), 76, 24));
        assert!(out.iter().any(|line| line.contains("Model catalogs refreshed.")));
    }

    /// A refresh finishing must not move the selection out from under a hand already on
    /// its way to pressing enter.
    #[test]
    fn a_refresh_keeps_the_reader_where_they_were() {
        let items = |extra: bool| {
            let mut items = vec![
                PickerItem::new("a-model", "[one]", "/model a"),
                PickerItem::new("b-model", "[two]", "/model b"),
            ];
            if extra {
                items.insert(0, PickerItem::new("new-model", "[three]", "/model n"));
            }
            items
        };
        let mut list = Picker::new(Choices::new("Select a model", items(false)));
        list.select_next();
        assert_eq!(list.selected_item().unwrap().command, "/model b");

        list.replace_items(Choices::new("Select a model", items(true)));
        assert_eq!(
            list.selected_item().unwrap().command,
            "/model b",
            "the same row, though another was added above it"
        );
    }

    /// A question put by an extension is not one the reader opened, so it says what it is
    /// and which keys answer it.
    #[test]
    fn a_question_names_itself_and_says_which_keys_answer_it() {
        let asked = Picker::new(
            Choices::new(
                "Overwrite the file?",
                vec![
                    PickerItem::new("Yes", "", "yes"),
                    PickerItem::new("No", "", "no"),
                ],
            )
            .titled(),
        );
        let out = rendered(&picker_lines(&asked, &Theme::dark(), 70, 20));
        assert!(out.iter().any(|line| line.contains("Overwrite the file?")));
        assert!(out.iter().any(|line| line.contains("esc cancel")));
    }

    #[test]
    fn the_filter_shows_what_has_been_typed() {
        let mut picker = picker();
        picker.push("gem");
        let out = rendered(&picker_lines(&picker, &Theme::dark(), 70, 20));
        assert!(out.iter().any(|line| line == "  > gem"), "{out:?}");
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
        assert!(out.last().unwrap().starts_with('─'), "and framed below");
        assert!(
            out.iter().any(|line| line.contains("> zzz")),
            "the query stays on screen so it can be corrected: {out:?}"
        );
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
