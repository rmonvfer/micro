use crate::app::KeyPrompt;
use crate::editor::Editor;
use crate::picker::Picker;
use crate::picker::MAX_VISIBLE;
use crate::render::clip;
use crate::render::editor::first_visible_row;
use crate::render::hints;
use crate::render::tint;
use crate::theme::Theme;
use crate::wrap::grapheme_width;
use crate::wrap::text_width;
use crate::wrap::truncate;
use crate::wrap::wrap_spans;
use micro_commands::InspectionItem;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

/// Columns the selection arrow occupies.
const MARKER_WIDTH: usize = 2;
/// Gap between an item's label and its detail.
const COLUMN_GAP: usize = 2;
/// Below this width a row has no room for two columns.
const NARROW: usize = 40;
/// A detail shorter than this says nothing, so it is left off.
const MIN_DETAIL: usize = 10;

pub struct Inspection<'a> {
    pub title: &'a str,
    pub items: &'a [InspectionItem],
    pub body: &'a [Line<'static>],
    pub selected: usize,
    pub detail_open: bool,
    pub scroll: usize,
    pub theme: &'a Theme,
    pub width: usize,
    pub budget: usize,
}

pub fn inspection_lines(inspection: Inspection<'_>) -> Vec<Line<'static>> {
    let Inspection {
        title,
        items,
        body,
        selected,
        detail_open,
        scroll,
        theme,
        width,
        budget,
    } = inspection;

    if detail_open {
        if let Some(item) = items.get(selected) {
            return inspection_text_lines(
                &format!("{title} · {}", item.label),
                body,
                scroll,
                theme,
                width,
                budget,
                "↑↓ scroll · esc back",
            );
        }
    }
    if items.is_empty() {
        return inspection_text_lines(
            title,
            body,
            scroll,
            theme,
            width,
            budget,
            "↑↓ scroll · esc close",
        );
    }

    let mut out = inspection_title(title, theme, width);
    let tail = inspection_tail("↑↓ navigate · enter inspect · esc close", theme, width);
    let summary_budget = budget
        .saturating_sub(out.len() + tail.len() + usize::from(!body.is_empty()))
        .min((budget / 3).max(1));
    out.extend(body.iter().take(summary_budget).cloned());
    if !body.is_empty() {
        out.push(Line::default());
    }

    let tail = inspection_tail("↑↓ navigate · enter inspect · esc close", theme, width);
    let row_budget = budget.saturating_sub(out.len() + tail.len()).max(1);
    let first = selected
        .saturating_sub(row_budget / 2)
        .min(items.len().saturating_sub(row_budget));
    for (index, item) in items.iter().enumerate().skip(first).take(row_budget) {
        let marker = if index == selected { "› " } else { "  " };
        let style = if index == selected {
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            theme.body()
        };
        out.push(clip(
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(item.label.clone(), style),
            ]),
            width,
        ));
    }
    out.extend(tail);
    out.truncate(budget.max(1));
    out.into_iter()
        .map(|line| tint(line, width, theme.surface))
        .collect()
}

fn inspection_text_lines(
    title: &str,
    body: &[Line<'static>],
    scroll: usize,
    theme: &Theme,
    width: usize,
    budget: usize,
    hint_text: &str,
) -> Vec<Line<'static>> {
    let head = inspection_title(title, theme, width);
    let tail = inspection_tail(hint_text, theme, width);
    let body_height = budget.saturating_sub(head.len() + tail.len()).max(1);
    let first = scroll.min(body.len().saturating_sub(body_height));
    let mut out = head;
    out.extend(body.iter().skip(first).take(body_height).cloned());
    out.extend(tail);
    out.truncate(budget.max(1));
    out.into_iter()
        .map(|line| tint(line, width, theme.surface))
        .collect()
}

fn inspection_title(title: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    vec![
        rule(theme, width),
        Line::default(),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                title.to_string(),
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::default(),
    ]
}

fn inspection_tail(text: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    vec![
        Line::default(),
        hint(text, theme),
        Line::default(),
        rule(theme, width),
    ]
}

pub fn picker_lines(
    picker: &Picker,
    theme: &Theme,
    width: usize,
    max_rows: usize,
) -> Vec<Line<'static>> {
    let mut head = vec![rule(theme, width)];
    let plain = !picker.searchable() && !picker.titled();

    if !plain {
        head.push(Line::default());

        match picker.hint() {
            Some(saying) => head.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(saying.to_string(), Style::new().fg(theme.warning)),
            ])),
            None => head.push(title(picker, theme)),
        }

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

    if let Some((text, ok)) = picker.status() {
        let style = match ok {
            true => Style::new().fg(theme.success),
            false => theme.dimmed(),
        };
        tail.insert(0, Line::default());
        tail.insert(
            1,
            Line::from(vec![Span::styled(format!("  {text}"), style)]),
        );
    }

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
    head.into_iter().map(|line| clip(line, width)).collect()
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
fn filter(picker: &Picker, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled("  > ", Style::new().fg(theme.accent)),
        Span::styled(picker.query().to_string(), Style::new().fg(theme.text)),
        Span::styled(
            " ",
            Style::new().fg(theme.text).add_modifier(Modifier::REVERSED),
        ),
    ])
}

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

    let available = width.saturating_sub(MARKER_WIDTH + 2);

    let column = match width > NARROW {
        true => column,
        false => 0,
    };

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

/// The prompt for a credential.
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

fn split_at_cursor(text: &str, column: usize) -> (String, Option<String>, String) {
    let mut seen = 0;
    for (index, grapheme) in text.grapheme_indices(true) {
        if seen >= column {
            let after = &text[index + grapheme.len()..];
            return (
                text[..index].to_string(),
                Some(grapheme.to_string()),
                after.to_string(),
            );
        }
        seen += grapheme_width(grapheme);
    }
    (text.to_string(), None, String::new())
}

pub fn extension_editor_lines(
    title: &str,
    editor: &Editor,
    theme: &Theme,
    width: usize,
    budget: usize,
) -> Vec<Line<'static>> {
    let indent = 2;
    let mut out = vec![
        Line::from(vec![
            Span::raw(" ".repeat(indent)),
            Span::styled(
                title.to_string(),
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::default(),
    ];

    let body_height = budget.saturating_sub(4).max(1);
    let body_width = width.saturating_sub(indent).max(1);
    let layout = editor.layout(body_width);
    let first = first_visible_row(layout.cursor_row, layout.rows.len(), body_height);

    let text = Style::new().fg(theme.text);
    let cursor = Style::new().fg(theme.text).add_modifier(Modifier::REVERSED);
    for (index, row) in layout.rows.iter().enumerate().skip(first).take(body_height) {
        let source = &editor.lines()[row.line][row.range.clone()];
        let mut spans = vec![Span::raw(" ".repeat(indent))];
        if index == layout.cursor_row {
            let (before, at, after) = split_at_cursor(source, layout.cursor_column);
            spans.push(Span::styled(before, text));
            spans.push(Span::styled(at.unwrap_or_else(|| " ".to_string()), cursor));
            if !after.is_empty() {
                spans.push(Span::styled(after, text));
            }
        } else {
            spans.push(Span::styled(source.to_string(), text));
        }
        out.push(Line::from(spans));
    }

    out.push(Line::default());
    out.push(hint(
        "enter submit · shift+enter newline · esc cancel",
        theme,
    ));

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

    /// A list that is searched says what it is and gives its query line room; the rule below it is
    /// the prompt's own.
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

    #[test]
    fn an_inspection_keeps_the_pickers_top_and_bottom_padding() {
        let body = vec![Line::styled("  summary", Theme::dark().body())];
        let theme = Theme::dark();
        let out = rendered(&inspection_lines(Inspection {
            title: "Session bill",
            items: &[],
            body: &body,
            selected: 0,
            detail_open: false,
            scroll: 0,
            theme: &theme,
            width: 60,
            budget: 16,
        }));

        assert!(
            out.first().is_some_and(|line| line.starts_with('─')),
            "{out:?}"
        );
        assert_eq!(out[1], "", "space after the top rule: {out:?}");
        assert_eq!(
            out[out.len() - 2],
            "",
            "space before the bottom rule: {out:?}"
        );
        assert!(
            out.last().is_some_and(|line| line.starts_with('─')),
            "{out:?}"
        );
    }

    #[test]
    fn an_inspection_never_outgrows_its_budget_or_width() {
        let body = (0..100)
            .map(|index| Line::styled(format!("  line {index}"), Theme::dark().body()))
            .collect::<Vec<_>>();
        for width in 12..80 {
            for rows in 4..20 {
                let theme = Theme::dark();
                let out = inspection_lines(Inspection {
                    title: "Session bill",
                    items: &[],
                    body: &body,
                    selected: 0,
                    detail_open: false,
                    scroll: 50,
                    theme: &theme,
                    width,
                    budget: rows,
                });
                assert!(out.len() <= rows, "{} rows exceed {rows}", out.len());
                for line in out {
                    let drawn: usize = line
                        .spans
                        .iter()
                        .map(|span| text_width(&span.content))
                        .sum();
                    assert!(drawn <= width, "{drawn} columns exceed {width}");
                }
            }
        }
    }

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
        assert!(out
            .iter()
            .any(|line| line.contains("Model catalogs refreshed.")));
    }

    /// A refresh finishing must not move the selection out from under a hand already on its way to
    /// pressing enter.
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

    /// A question put by an extension is not one the reader opened, so it says what it is and which
    /// keys answer it.
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
                    assert!(drawn <= width, "{drawn} columns exceed {width}");
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

    #[test]
    fn an_extension_editor_names_itself_and_carries_the_prefill() {
        let mut editor = Editor::new();
        editor.set_text("draft text");
        let out = rendered(&extension_editor_lines(
            "Write a title",
            &editor,
            &Theme::dark(),
            70,
            20,
        ));

        assert!(out[0].contains("Write a title"));
        assert!(out.iter().any(|line| line.contains("draft text")));
        assert!(out.iter().any(|line| line.contains("submit")));
        assert!(out.iter().any(|line| line.contains("newline")));
        assert!(out.iter().any(|line| line.contains("cancel")));
    }

    /// The cursor lands where `Editor::layout` says it does, mid-line included.
    #[test]
    fn the_cursor_marks_wherever_it_sits_in_the_text() {
        let mut editor = Editor::new();
        editor.set_text("ab");
        editor.move_start();
        editor.move_right();

        let out = extension_editor_lines("Title", &editor, &Theme::dark(), 70, 20);
        let body = &out[2];
        let marked: Vec<&Span> = body
            .spans
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .collect();

        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].content.as_ref(), "b");
    }

    #[test]
    fn a_long_editor_never_outgrows_its_budget_or_its_width() {
        let mut editor = Editor::new();
        editor.set_text(&"a very long line that wraps several times over ".repeat(10));

        for width in 12..80 {
            for rows in 5..20 {
                let out = extension_editor_lines("Title", &editor, &Theme::dark(), width, rows);
                assert!(out.len() <= rows, "{} rows exceed {rows}", out.len());
                for line in out {
                    let drawn: usize = line.spans.iter().map(|s| text_width(&s.content)).sum();
                    assert!(drawn <= width, "{drawn} columns exceed {width}");
                }
            }
        }
    }
}
