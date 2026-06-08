//! The input area: a rule above, a rule below, and the prompt between them.

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

/// Draw the input and the rules around it.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    /// Colour of the rules: the reasoning effort is marked here, so a raised level is visible
    /// without reading the footer.
    pub level: ratatui::style::Color,
    /// Whether the input has the keyboard.
    pub focused: bool,
    /// Whether the terminal draws the cursor itself.
    pub hardware_cursor: bool,
}

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    content: Rect,
    editor: &Editor,
    theme: &Theme,
    look: Look,
) {
    let Look {
        level,
        focused,
        hardware_cursor,
    } = look;
    if area.height == 0 || content.width == 0 {
        return;
    }

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

            let line = Line::from(vec![Span::styled(source.replace('\t', " "), text)]);
            frame
                .buffer_mut()
                .set_line(rows.x, rows.y + offset as u16, &line, rows.width);
        }
    }

    if focused {
        let cursor_row = layout.cursor_row.saturating_sub(first).min(height - 1);
        if !hardware_cursor {
            let column = rows.x + (layout.cursor_column as u16).min(rows.width.saturating_sub(1));
            let position = (column, rows.y + cursor_row as u16);
            frame.buffer_mut().cell_mut(position).map(|cell| {
                cell.set_style(Style::new().add_modifier(ratatui::style::Modifier::REVERSED))
            });
            return;
        }
        frame.set_cursor_position((
            rows.x + (layout.cursor_column as u16).min(rows.width.saturating_sub(1)),
            rows.y + cursor_row as u16,
        ));
    }
}

/// Scroll the input just enough to keep the cursor on screen.
pub(super) fn first_visible_row(cursor_row: usize, total: usize, height: usize) -> usize {
    if total <= height {
        return 0;
    }
    cursor_row
        .saturating_sub(height.saturating_sub(1))
        .min(total - height)
}

pub fn draw_component(
    frame: &mut Frame,
    area: Rect,
    content: Rect,
    lines: &[String],
    cursor: Option<(usize, usize)>,
    theme: &Theme,
    level: ratatui::style::Color,
) {
    if area.height == 0 || content.width == 0 {
        return;
    }
    let bar = Line::from(vec![Span::styled(
        "─".repeat(area.width as usize),
        Style::new().fg(level),
    )]);
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
    let text = Style::new().fg(theme.text);
    for (offset, source) in lines.iter().take(rows.height as usize).enumerate() {
        let line = Line::from(vec![Span::styled(source.clone(), text)]);
        frame
            .buffer_mut()
            .set_line(rows.x, rows.y + offset as u16, &line, rows.width);
    }
    if let Some((row, column)) = cursor {
        if row < rows.height as usize {
            frame.set_cursor_position((
                rows.x + (column as u16).min(rows.width.saturating_sub(1)),
                rows.y + row as u16,
            ));
        }
    }
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
            .draw(|frame| {
                draw(
                    frame,
                    area,
                    content,
                    editor,
                    &Theme::dark(),
                    Look {
                        level: Theme::dark().border_muted,
                        focused: true,
                        hardware_cursor: true,
                    },
                )
            })
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
                    Look {
                        level: theme.border_muted,
                        focused: false,
                        hardware_cursor: true,
                    },
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
