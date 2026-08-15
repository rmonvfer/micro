//! The slash-command menu, drawn under the input.
//!
//! An arrow marks the highlighted row, the command names sit in a column of their own,
//! descriptions fill what is left, and a count appears when the list is longer than the
//! window.

use crate::menu::Menu;
#[cfg(test)]
use crate::menu::MAX_VISIBLE;
use crate::theme::Theme;
use crate::wrap::text_width;
use crate::wrap::truncate;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

/// Below this the description column is dropped and only names are shown.
const MIN_WIDTH_FOR_DESCRIPTIONS: usize = 40;
/// Columns the selection arrow occupies. It is three bytes wide but two columns.
const MARKER_WIDTH: usize = 2;
/// Gap between the name column and the descriptions.
const COLUMN_GAP: usize = 2;

pub fn lines(menu: &Menu, theme: &Theme, width: usize, rows: usize) -> Vec<Line<'static>> {
    let window = menu.window(rows);
    let names = menu.items()[window.clone()]
        .iter()
        .map(|item| text_width(&item.value))
        .max()
        .unwrap_or(0);

    let mut out: Vec<Line<'static>> = window
        .clone()
        .map(|index| {
            let item = &menu.items()[index];
            row(
                &item.value,
                &item.description,
                index == menu.selected(),
                names,
                theme,
                width,
            )
        })
        .collect();

    // The count only earns its row when the window is hiding something.
    if window.len() < menu.items().len() {
        out.push(super::clip(
            Line::from(vec![Span::styled(
                format!("  ({}/{})", menu.selected() + 1, menu.items().len()),
                theme.secondary(),
            )]),
            width,
        ));
    }
    out
}

fn row(
    value: &str,
    description: &str,
    selected: bool,
    column: usize,
    theme: &Theme,
    width: usize,
) -> Line<'static> {
    let marker = match selected {
        true => "→ ",
        false => "  ",
    };
    // The highlighted row runs whole through `accent` — description included — and every
    // other row stays in the default foreground with its description in `muted`.
    let name = match selected {
        true => Style::new().fg(theme.accent),
        false => Style::new().fg(theme.text),
    };
    let description_style = match selected {
        true => Style::new().fg(theme.accent),
        false => theme.secondary(),
    };

    let available = width.saturating_sub(MARKER_WIDTH);
    let column = column.min(available.saturating_sub(1));
    let shown = truncate(value, column);

    let mut spans = vec![
        Span::styled(marker, Style::new().fg(theme.accent)),
        Span::styled(shown.clone(), name),
    ];

    let used = MARKER_WIDTH + column + COLUMN_GAP;
    let remaining = width.saturating_sub(used);
    if !description.is_empty() && width >= MIN_WIDTH_FOR_DESCRIPTIONS && remaining > 0 {
        let padding = column - text_width(&shown) + COLUMN_GAP;
        spans.push(Span::raw(" ".repeat(padding)));
        spans.push(Span::styled(
            truncate(description, remaining),
            description_style,
        ));
    }
    super::clip(Line::from(spans), width)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_highlighted_row_carries_the_arrow() {
        let mut menu = Menu::open_for("/c", 2, &[]).unwrap();
        let out = rendered(&lines(&menu, &Theme::dark(), 60, MAX_VISIBLE));
        assert!(out[0].starts_with("→ clone"));
        assert!(out[1].starts_with("  changelog"));

        menu.select_next();
        let out = rendered(&lines(&menu, &Theme::dark(), 60, MAX_VISIBLE));
        assert!(out[0].starts_with("  clone"));
        assert!(out[1].starts_with("→ changelog"));
    }

    #[test]
    fn names_line_up_in_a_column_with_their_descriptions() {
        let menu = Menu::open_for("/c", 2, &[]).unwrap();
        let out = rendered(&lines(&menu, &Theme::dark(), 70, MAX_VISIBLE));
        let column_of = |line: &str, text: &str| {
            let byte = line.find(text).expect("a description");
            text_width(&line[..byte])
        };
        assert_eq!(
            column_of(&out[3], "summarize the conversation"),
            column_of(&out[4], "start a fresh conversation"),
            "descriptions share a column"
        );
    }

    #[test]
    fn a_narrow_menu_drops_the_descriptions_rather_than_wrapping() {
        let menu = Menu::open_for("/c", 2, &[]).unwrap();
        for line in lines(&menu, &Theme::dark(), 20, MAX_VISIBLE) {
            let width: usize = line.spans.iter().map(|s| text_width(&s.content)).sum();
            assert!(width <= 20, "row of {width} exceeds 20");
        }
        let out = rendered(&lines(&menu, &Theme::dark(), 20, MAX_VISIBLE));
        assert_eq!(out[0], "→ clone");
    }

    #[test]
    fn no_row_ever_exceeds_the_width() {
        let menu = Menu::open_for("/", 1, &[]).unwrap();
        for width in 4..90 {
            for line in lines(&menu, &Theme::dark(), width, MAX_VISIBLE) {
                let drawn: usize = line.spans.iter().map(|s| text_width(&s.content)).sum();
                assert!(drawn <= width, "row of {drawn} exceeds {width}");
            }
        }
    }

    #[test]
    fn a_scrolling_list_reports_where_you_are() {
        let mut menu = Menu::open_for("/", 1, &[]).unwrap();
        let total = menu.items().len();
        let out = rendered(&lines(&menu, &Theme::dark(), 60, MAX_VISIBLE));
        assert_eq!(out.last().unwrap(), &format!("  (1/{total})"));
        assert_eq!(out.len(), MAX_VISIBLE + 1);

        menu.select_next();
        let out = rendered(&lines(&menu, &Theme::dark(), 60, MAX_VISIBLE));
        assert_eq!(out.last().unwrap(), &format!("  (2/{total})"));
    }

    #[test]
    fn a_list_that_fits_has_no_count() {
        let menu = Menu::open_for("/com", 4, &[]).unwrap();
        let out = rendered(&lines(&menu, &Theme::dark(), 60, MAX_VISIBLE));
        assert_eq!(out.len(), 1);
    }
}
