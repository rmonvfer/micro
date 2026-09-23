//! In-app mouse text selection.

use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use ratatui::style::Style;

/// An active mouse text selection across terminal coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Where the drag began (column, row).
    pub origin: (u16, u16),
    /// Where the drag currently is (column, row).
    pub current: (u16, u16),
    /// True while the mouse button is held down.
    pub dragging: bool,
    /// Set on mouse release so the next render pass copies the text to the clipboard.
    pub copy_pending: bool,
}

impl Selection {
    pub fn new(col: u16, row: u16) -> Self {
        Self {
            origin: (col, row),
            current: (col, row),
            dragging: true,
            copy_pending: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.origin == self.current
    }

    /// Normalized (start, end) where start <= end in reading order.
    pub fn range(&self) -> ((u16, u16), (u16, u16)) {
        let (x0, y0) = self.origin;
        let (x1, y1) = self.current;
        if y0 < y1 || (y0 == y1 && x0 <= x1) {
            ((x0, y0), (x1, y1))
        } else {
            ((x1, y1), (x0, y0))
        }
    }
}

/// Extract selected text from the terminal buffer, trimming trailing line padding.
pub fn extract_text(buffer: &Buffer, selection: &Selection) -> String {
    if selection.is_empty() {
        return String::new();
    }
    let area = buffer.area;
    if area.width == 0 || area.height == 0 {
        return String::new();
    }

    let ((start_x, start_y), (end_x, end_y)) = selection.range();
    if start_y >= area.height {
        return String::new();
    }
    let min_y = start_y;
    let max_y = end_y.min(area.height.saturating_sub(1));

    let mut lines = Vec::new();
    for y in min_y..=max_y {
        let (min_x, max_x) = if start_y == end_y {
            (start_x, end_x.min(area.width.saturating_sub(1)))
        } else if y == start_y {
            (start_x, area.width.saturating_sub(1))
        } else if y == end_y {
            (0, end_x.min(area.width.saturating_sub(1)))
        } else {
            (0, area.width.saturating_sub(1))
        };

        if min_x > max_x {
            lines.push(String::new());
            continue;
        }

        let mut line = String::new();
        for x in min_x..=max_x {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }

    while lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

/// Highlight selected cells in the terminal buffer by inverting their style.
pub fn apply_selection(buffer: &mut Buffer, selection: &Selection) {
    if selection.is_empty() {
        return;
    }
    let area = buffer.area;
    if area.width == 0 || area.height == 0 {
        return;
    }

    let ((start_x, start_y), (end_x, end_y)) = selection.range();
    if start_y >= area.height {
        return;
    }
    let min_y = start_y;
    let max_y = end_y.min(area.height.saturating_sub(1));

    for y in min_y..=max_y {
        let last_col = (0..area.width)
            .rposition(|x| buffer[(x, y)].symbol() != " ")
            .map(|x| x as u16);

        let Some(last) = last_col else {
            continue;
        };

        let (min_x, max_x) = if start_y == end_y {
            if start_x > last && end_x > last {
                continue;
            }
            let from = start_x.min(last);
            let to = end_x.min(last);
            (from, to)
        } else if y == start_y {
            if start_x > last {
                continue;
            }
            (start_x, last)
        } else if y == end_y {
            (0, end_x.min(last))
        } else {
            (0, last)
        };

        if min_x <= max_x {
            for x in min_x..=max_x {
                let cell = &mut buffer[(x, y)];
                cell.set_style(Style::new().add_modifier(Modifier::REVERSED));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn sample_buffer(width: u16, height: u16, text_lines: &[&str]) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buffer = Buffer::empty(area);
        for (y, line) in text_lines.iter().enumerate() {
            if (y as u16) < height {
                buffer.set_string(0, y as u16, line, Style::default());
            }
        }
        buffer
    }

    #[test]
    fn selection_range_normalizes_forward_and_backward() {
        let forward = Selection {
            origin: (5, 2),
            current: (15, 4),
            dragging: false,
            copy_pending: false,
        };
        assert_eq!(forward.range(), ((5, 2), (15, 4)));

        let backward = Selection {
            origin: (15, 4),
            current: (5, 2),
            dragging: false,
            copy_pending: false,
        };
        assert_eq!(backward.range(), ((5, 2), (15, 4)));

        let same_row_backward = Selection {
            origin: (20, 3),
            current: (10, 3),
            dragging: false,
            copy_pending: false,
        };
        assert_eq!(same_row_backward.range(), ((10, 3), (20, 3)));
    }

    #[test]
    fn single_line_selection_extracts_exact_text() {
        let buffer = sample_buffer(40, 5, &["Hello, world!", "Second line"]);
        let mut selection = Selection::new(0, 0);
        selection.current = (4, 0);
        assert_eq!(extract_text(&buffer, &selection), "Hello");

        let mut mid_selection = Selection::new(7, 0);
        mid_selection.current = (11, 0);
        assert_eq!(extract_text(&buffer, &mid_selection), "world");
    }

    #[test]
    fn multi_line_selection_extracts_and_trims_padding() {
        let buffer = sample_buffer(40, 5, &["Line one", "Line two is longer", "Line three"]);
        let mut selection = Selection::new(5, 0);
        selection.current = (4, 2);
        let text = extract_text(&buffer, &selection);
        assert_eq!(text, "one\nLine two is longer\nLine");
    }

    #[test]
    fn empty_selection_extracts_empty_string() {
        let buffer = sample_buffer(40, 5, &["Hello"]);
        let selection = Selection::new(2, 0);
        assert_eq!(extract_text(&buffer, &selection), "");
    }

    #[test]
    fn apply_selection_reverses_only_selected_cells() {
        let mut buffer = sample_buffer(20, 3, &["Hello world"]);
        let mut selection = Selection::new(0, 0);
        selection.current = (4, 0);

        apply_selection(&mut buffer, &selection);

        for x in 0..=4 {
            assert!(
                buffer[(x, 0)].modifier.contains(Modifier::REVERSED),
                "cell {x} should be reversed"
            );
        }
        for x in 5..20 {
            assert!(
                !buffer[(x, 0)].modifier.contains(Modifier::REVERSED),
                "cell {x} should not be reversed"
            );
        }
    }

    #[test]
    fn apply_selection_does_not_highlight_void_beyond_text() {
        let mut buffer = sample_buffer(30, 2, &["Short"]);
        let mut selection = Selection::new(10, 0);
        selection.current = (25, 0);

        apply_selection(&mut buffer, &selection);

        for x in 0..30 {
            assert!(
                !buffer[(x, 0)].modifier.contains(Modifier::REVERSED),
                "void cell {x} should not be reversed"
            );
        }
    }
}
