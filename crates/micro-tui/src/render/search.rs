//! The transcript search, drawn: its matches marked where they are, and the box holding its query.

use crate::app::App;
use crate::render::hints::key_text;
use crate::search::Search;
use crate::theme::Theme;
use crate::wrap::text_width;
use crate::wrap::truncate;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::Frame;

/// The narrowest the search box is drawn.
const MIN_BOX_WIDTH: u16 = 32;
/// What the query line says before anything is typed.
const PLACEHOLDER: &str = "Find in transcript";

/// Mark every match in view, and the selected one most of all. Each cell is styled on its own, so
/// nothing reaches past the last character of a match.
pub fn highlight(frame: &mut Frame, area: Rect, app: &App, search: &Search, theme: &Theme) {
    let rows = app.lines();
    let height = area.height as usize;
    if rows.is_empty() || height == 0 {
        return;
    }
    let first = rows
        .len()
        .saturating_sub(height)
        .saturating_sub(app.scroll());
    let shown = rows.len().saturating_sub(first).min(height);
    let top = area.y as usize;

    let other = Style::new()
        .bg(theme.selected_bg)
        .add_modifier(Modifier::UNDERLINED);
    let selected = Style::new()
        .fg(theme.accent)
        .add_modifier(Modifier::REVERSED | Modifier::BOLD);

    for (index, found) in search.matches().iter().enumerate() {
        if found.last_row() < first || found.first_row() >= first + shown {
            continue;
        }
        let style = match search.selected() == Some(index) {
            true => selected,
            false => other,
        };
        for segment in &found.segments {
            if segment.row < first || segment.row >= first + shown {
                continue;
            }
            let y = (top + segment.row - first) as u16;
            let end = segment.end.min(area.width as usize);
            for column in segment.start..end {
                frame.buffer_mut()[(area.x + column as u16, y)].set_style(style);
            }
        }
    }
}

/// The box at the top right of the transcript: the query, how many matches it has, and the keys
/// that step between them.
pub fn draw_box(frame: &mut Frame, area: Rect, search: &Search, theme: &Theme) {
    if area.height < 3 || area.width < 4 {
        return;
    }
    let width = (area.width * 2 / 5).max(MIN_BOX_WIDTH).min(area.width);
    let inner = width as usize - 2;
    let x = area.x + area.width - width;
    let y = area.y;

    let border = Style::new().fg(theme.border_accent).bg(theme.surface);
    let ground = Style::new().bg(theme.surface);
    let dim = Style::new().fg(theme.dim).bg(theme.surface);
    let body = Style::new().fg(theme.text).bg(theme.surface);
    let buffer = frame.buffer_mut();

    buffer.set_string(x, y, format!("┌{}┐", "─".repeat(inner)), border);

    let result = match (search.query().trim().is_empty(), search.selected()) {
        (true, _) => String::new(),
        (false, None) => " No matches ".to_string(),
        (false, Some(index)) => format!(" {}/{} ", index + 1, search.matches().len()),
    };
    let result = truncate(&result, inner.saturating_sub(3));
    let room = inner.saturating_sub(text_width(&result) + 2);
    buffer.set_string(x, y + 1, "│", border);
    buffer.set_string(x + 1, y + 1, " ".repeat(inner), ground);
    let (query, style) = match search.query().is_empty() {
        true => (PLACEHOLDER.to_string(), dim),
        false => (search.query().to_string(), body),
    };
    let query = tail(&query, room);
    buffer.set_string(x + 2, y + 1, &query, style);
    let cursor = x + 2 + text_width(&query).min(room) as u16;
    if cursor < x + width - 1 {
        buffer[(cursor, y + 1)].set_style(body.add_modifier(Modifier::REVERSED));
    }
    let result_x = x + 1 + (inner - text_width(&result)) as u16;
    buffer.set_string(result_x, y + 1, &result, dim);
    buffer.set_string(x + width - 1, y + 1, "│", border);

    let controls = format!(" ↑ {} · ↓ {} ", key_text("shift+enter"), key_text("enter"));
    let bottom = match text_width(&controls) + 2 <= inner {
        true => {
            let rule = inner - text_width(&controls) - 1;
            format!("└{}{controls}─┘", "─".repeat(rule))
        }
        false => format!("└{}┘", "─".repeat(inner)),
    };
    buffer.set_string(x, y + 2, bottom, border);
}

/// The end of `text` that fits in `width` columns, so the cursor end of a long query stays in view.
fn tail(text: &str, width: usize) -> String {
    let mut kept: Vec<char> = Vec::new();
    let mut used = 0;
    for character in text.chars().rev() {
        let size = text_width(&character.to_string());
        if used + size > width {
            break;
        }
        used += size;
        kept.push(character);
    }
    kept.into_iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TuiOptions;
    use crate::event::Action;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn searched_app(query: &str) -> App {
        let mut app = App::new(&[], TuiOptions::default());
        app.set_tui_mode(crate::TuiMode::Fullscreen);
        app.transcript.push_user("alpha needle omega");
        app.transcript.push_user("another needle here");
        app.handle(Action::Find);
        app.handle(Action::Insert(query.to_string()));
        app
    }

    fn draw(app: &mut App) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(70, 20)).expect("backend");
        terminal
            .draw(|frame| crate::render::draw(frame, app))
            .expect("draws");
        terminal
    }

    fn row_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn matches_are_marked_cell_by_cell_and_stop_at_their_last_character() {
        let mut app = searched_app("needle");
        let theme = app.theme;
        let terminal = draw(&mut app);
        let buffer = terminal.backend().buffer();

        let row = (0..buffer.area.height)
            .find(|y| row_text(&terminal, *y).contains("alpha needle"))
            .expect("the prompt is drawn");
        let start = row_text(&terminal, row).find("needle").unwrap() as u16;

        for x in start..start + 6 {
            let cell = &buffer[(x, row)];
            assert!(
                cell.bg == theme.selected_bg || cell.modifier.contains(Modifier::REVERSED),
                "cell {x} is marked"
            );
        }
        let after = &buffer[(start + 6, row)];
        assert_eq!(after.symbol(), " ");
        assert!(!after.modifier.contains(Modifier::REVERSED));
        assert!(!after.modifier.contains(Modifier::UNDERLINED));
        assert_ne!(
            after.bg, theme.selected_bg,
            "the mark does not bleed past the match"
        );
    }

    #[test]
    fn the_box_shows_the_query_and_where_the_selection_is() {
        let mut app = searched_app("needle");
        let terminal = draw(&mut app);
        let top = row_text(&terminal, 1);
        assert!(top.contains("needle"), "{top}");
        assert!(top.contains("/2"), "{top}");

        let mut app = searched_app("nothing like it");
        let terminal = draw(&mut app);
        assert!(row_text(&terminal, 1).contains("No matches"));
    }

    #[test]
    fn a_long_query_shows_its_end() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("ab", 3), "ab");
    }
}
