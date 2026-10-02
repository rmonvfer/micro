//! In-app mouse text selection.

use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use ratatui::style::Style;

/// How far a selection reaches past the cells the pointer covered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Granularity {
    /// Exactly the cells dragged across.
    #[default]
    Character,
    /// Out to the whole words at either end, as a double click selects.
    Word,
    /// Out to the blank rows around it, as a triple click selects.
    Paragraph,
}

impl Granularity {
    /// What the `count`th click in a row at the same place selects.
    pub fn for_clicks(count: u8) -> Self {
        match count {
            2 => Granularity::Word,
            3 => Granularity::Paragraph,
            _ => Granularity::Character,
        }
    }
}

/// Why the selected text is about to be copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyRequest {
    /// The mouse was released over it, and selections copy themselves.
    Released,
    /// The copy key was pressed while it was showing.
    Asked,
}

/// An active mouse text selection across terminal coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Where the drag began (column, row).
    pub origin: (u16, u16),
    /// Where the drag currently is (column, row).
    pub current: (u16, u16),
    /// True while the mouse button is held down.
    pub dragging: bool,
    /// Set when the next render pass should copy the text to the clipboard, and why.
    pub copy_pending: Option<CopyRequest>,
    /// How far past the covered cells it reaches.
    pub granularity: Granularity,
}

impl Selection {
    /// A plain selection starting at a point.
    #[cfg(test)]
    pub fn new(col: u16, row: u16) -> Self {
        Selection::at(col, row, Granularity::Character)
    }

    /// A selection starting at a point, reaching as far as `granularity` says.
    pub fn at(col: u16, row: u16, granularity: Granularity) -> Self {
        Self {
            origin: (col, row),
            current: (col, row),
            dragging: true,
            copy_pending: None,
            granularity,
        }
    }

    /// Whether it covers nothing: a plain click selects nothing, a double or triple click does.
    pub fn is_empty(&self) -> bool {
        self.origin == self.current && self.granularity == Granularity::Character
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

/// The cells a selection covers on this buffer, reaching out to whole words or paragraphs when it
/// was made by more than one click.
pub fn covered(buffer: &Buffer, selection: &Selection) -> ((u16, u16), (u16, u16)) {
    let ((start_x, start_y), (end_x, end_y)) = selection.range();
    let area = buffer.area;
    let last_x = area.width.saturating_sub(1);
    let last_y = area.height.saturating_sub(1);
    let (start_y, end_y) = (start_y.min(last_y), end_y.min(last_y));
    let (start_x, end_x) = (start_x.min(last_x), end_x.min(last_x));
    let blank = |x: u16, y: u16| buffer[(x, y)].symbol().trim().is_empty();
    let blank_row = |y: u16| (0..area.width).all(|x| blank(x, y));

    match selection.granularity {
        Granularity::Character => ((start_x, start_y), (end_x, end_y)),
        Granularity::Word => {
            let mut from = start_x;
            if !blank(from, start_y) {
                while from > 0 && !blank(from - 1, start_y) {
                    from -= 1;
                }
            }
            let mut to = end_x;
            if !blank(to, end_y) {
                while to < last_x && !blank(to + 1, end_y) {
                    to += 1;
                }
            }
            ((from, start_y), (to, end_y))
        }
        Granularity::Paragraph => {
            let mut top = start_y;
            while top > 0 && !blank_row(top) && !blank_row(top - 1) {
                top -= 1;
            }
            let mut bottom = end_y;
            while bottom < last_y && !blank_row(bottom) && !blank_row(bottom + 1) {
                bottom += 1;
            }
            ((0, top), (last_x, bottom))
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

    let ((start_x, start_y), (end_x, end_y)) = covered(buffer, selection);
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

    let ((start_x, start_y), (end_x, end_y)) = covered(buffer, selection);
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
            copy_pending: None,
            granularity: Granularity::Character,
        };
        assert_eq!(forward.range(), ((5, 2), (15, 4)));

        let backward = Selection {
            origin: (15, 4),
            current: (5, 2),
            dragging: false,
            copy_pending: None,
            granularity: Granularity::Character,
        };
        assert_eq!(backward.range(), ((5, 2), (15, 4)));

        let same_row_backward = Selection {
            origin: (20, 3),
            current: (10, 3),
            dragging: false,
            copy_pending: None,
            granularity: Granularity::Character,
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
    fn a_double_click_takes_the_whole_word_under_it() {
        let buffer = sample_buffer(40, 3, &["open src/main.rs now", "next line"]);
        let selection = Selection::at(8, 0, Granularity::Word);
        assert!(!selection.is_empty());
        assert_eq!(extract_text(&buffer, &selection), "src/main.rs");
    }

    #[test]
    fn a_triple_click_takes_the_rows_between_blank_ones() {
        let buffer = sample_buffer(
            30,
            6,
            &["before", "", "first row", "second row", "", "after"],
        );
        let selection = Selection::at(3, 3, Granularity::Paragraph);
        assert_eq!(extract_text(&buffer, &selection), "first row\nsecond row");
    }

    #[test]
    fn the_click_count_picks_how_far_a_selection_reaches() {
        assert_eq!(Granularity::for_clicks(1), Granularity::Character);
        assert_eq!(Granularity::for_clicks(2), Granularity::Word);
        assert_eq!(Granularity::for_clicks(3), Granularity::Paragraph);
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
