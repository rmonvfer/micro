//! The input area: a rule above, a rule below, and the prompt between them.
//!
//! ohm frames its input with two horizontal rules rather than a box or a fill, and colors
//! them by the reasoning budget the next turn will run with — so the frame around what you
//! are typing tells you how hard the model is about to think. A `!` line takes the bash
//! colour instead, marking the mode before it is sent.

use crate::editor::Editor;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::Frame;

/// Rows the rules take between them: one above the prompt and one below it.
pub const RULES: u16 = 2;

const PLACEHOLDER: &str = "Ask anything - enter to send, shift+enter for a new line";

/// Draw the input and the rules around it. `focused` is false while something else owns the
/// keyboard, and the cursor then stays away rather than blinking where the next keystroke
/// will not land.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    content: Rect,
    editor: &Editor,
    theme: &Theme,
    // Colour of the rules: ohm marks the reasoning effort here, so a raised level is
    // visible without reading the footer.
    level: ratatui::style::Color,
    focused: bool,
) {
    if area.height == 0 || content.width == 0 {
        return;
    }

    // The rules run edge to edge, past the margin the prompt itself keeps, and take the
    // bash colour while a `!` line is being typed so the mode is visible before it is sent.
    let rule = match editor.text().starts_with('!') {
        true => Style::new().fg(theme.bash_mode),
        false => Style::new().fg(level),
    };
    let bar = Line::from(vec![Span::styled("─".repeat(area.width as usize), rule)]);
    frame
        .buffer_mut()
        .set_line(area.x, area.y, &bar, area.width);
    if area.height > 1 {
        frame
            .buffer_mut()
            .set_line(area.x, area.y + area.height - 1, &bar, area.width);
    }

    let rows = Rect {
        x: content.x,
        y: content.y.saturating_add(1),
        width: content.width,
        height: area.height.saturating_sub(RULES),
    };
    if rows.height == 0 {
        return;
    }

    let width = rows.width as usize;
    let height = rows.height as usize;
    let layout = editor.layout(width);
    let first = first_visible_row(layout.cursor_row, layout.rows.len(), height);

    if editor.is_empty() {
        frame.buffer_mut().set_line(
            rows.x,
            rows.y,
            &Line::from(vec![Span::styled(
                crate::wrap::truncate(PLACEHOLDER, width),
                Style::new().fg(theme.dim),
            )]),
            rows.width,
        );
    } else {
        let text = Style::new().fg(theme.text);
        for (offset, row) in layout.rows.iter().skip(first).take(height).enumerate() {
            let source = &editor.lines()[row.line][row.range.clone()];
            // Tabs are one column wide in the wrap math, so they are drawn that way too.
            let line = Line::from(vec![Span::styled(source.replace('\t', " "), text)]);
            frame
                .buffer_mut()
                .set_line(rows.x, rows.y + offset as u16, &line, rows.width);
        }
    }

    if focused {
        let cursor_row = layout.cursor_row.saturating_sub(first).min(height - 1);
        frame.set_cursor_position((
            rows.x + (layout.cursor_column as u16).min(rows.width.saturating_sub(1)),
            rows.y + cursor_row as u16,
        ));
    }
}

/// Scroll the input just enough to keep the cursor on screen.
fn first_visible_row(cursor_row: usize, total: usize, height: usize) -> usize {
    if total <= height {
        return 0;
    }
    cursor_row
        .saturating_sub(height.saturating_sub(1))
        .min(total - height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn a_short_prompt_is_not_scrolled() {
        assert_eq!(first_visible_row(0, 1, 5), 0);
        assert_eq!(first_visible_row(2, 3, 5), 0);
    }

    #[test]
    fn a_tall_prompt_keeps_the_cursor_on_the_last_row() {
        assert_eq!(first_visible_row(9, 10, 3), 7);
        assert_eq!(first_visible_row(3, 10, 3), 1);
        assert_eq!(first_visible_row(0, 10, 3), 0);
    }

    fn painted(width: u16, height: u16, editor: &Editor) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
        let area = Rect::new(0, 0, width, height);
        let content = Rect::new(1, 0, width - 2, height);
        terminal
            .draw(|frame| draw(frame, area, content, editor, &Theme::dark(), Theme::dark().border_muted, true))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn a_rule_runs_edge_to_edge_above_and_below_the_prompt() {
        let mut editor = Editor::new();
        editor.insert_str("hello");
        let rows = painted(12, 3, &editor);

        assert_eq!(rows[0], "────────────");
        assert_eq!(rows[1], " hello");
        assert_eq!(rows[2], "────────────");
    }

    #[test]
    fn the_rules_carry_the_muted_border_color() {
        let theme = Theme::dark();
        let mut terminal = Terminal::new(TestBackend::new(10, 3)).expect("test backend");
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    Rect::new(0, 0, 10, 3),
                    Rect::new(1, 0, 8, 3),
                    &Editor::new(),
                    &theme,
                    theme.border_muted,
                    false,
                )
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].fg, theme.border_muted);
        assert_eq!(buffer[(0, 2)].fg, theme.border_muted);
    }

    #[test]
    fn nothing_is_painted_outside_the_area() {
        let mut editor = Editor::new();
        editor.insert_str("one\ntwo\nthree");
        for height in 1..6u16 {
            for width in 3..14u16 {
                let rows = painted(width, height, &editor);
                assert_eq!(rows.len(), height as usize);
            }
        }
    }
}
