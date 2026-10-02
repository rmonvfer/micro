//! micro's mark: three squares stepping down in size, drawn with half blocks, two square pixels
//! to a cell.

use crate::theme::Theme;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Span;

/// The mark, one character a pixel. `a` is the accent, `b` the border colour, `c` the heading
/// colour and `.` is left empty. Each pair of rows is drawn as one row of cells: a large square
/// at the top left, then a smaller one, then the smallest, stepping down to the bottom right.
#[rustfmt::skip]
const PIXELS: [&str; 4] = [
    "aa..",
    "aa..",
    "..b.",
    "...c",
];

/// Columns the mark takes.
pub const WIDTH: usize = 4;

/// Rows the mark takes.
pub const HEIGHT: usize = PIXELS.len() / 2;

/// The colour a pixel is drawn in, or nothing for an empty one.
fn color(pixel: char, theme: &Theme) -> Option<Color> {
    match pixel {
        'a' => Some(theme.accent),
        'b' => Some(theme.border),
        'c' => Some(theme.md_heading),
        _ => None,
    }
}

/// One cell holding the pixel above and the pixel below it.
fn cell(top: Option<Color>, bottom: Option<Color>) -> Span<'static> {
    match (top, bottom) {
        (Some(top), Some(bottom)) if top == bottom => Span::styled("█", Style::new().fg(top)),
        (Some(top), Some(bottom)) => Span::styled("▀", Style::new().fg(top).bg(bottom)),
        (Some(top), None) => Span::styled("▀", Style::new().fg(top)),
        (None, Some(bottom)) => Span::styled("▄", Style::new().fg(bottom)),
        (None, None) => Span::raw(" "),
    }
}

/// The mark's rows of cells, in the theme's colours.
pub fn rows(theme: &Theme) -> Vec<Vec<Span<'static>>> {
    PIXELS
        .chunks(2)
        .map(|pair| {
            pair[0]
                .chars()
                .zip(pair[1].chars())
                .map(|(top, bottom)| cell(color(top, theme), color(bottom, theme)))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrap::text_width;

    fn text(row: &[Span<'static>]) -> String {
        row.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn the_mark_is_four_cells_wide_and_two_rows_tall() {
        let rows = rows(&Theme::dark());
        let drawn: Vec<String> = rows.iter().map(|row| text(row)).collect();
        assert_eq!(drawn, ["██  ", "  ▀▄"]);
        assert_eq!(rows.len(), HEIGHT);
        assert!(drawn.iter().all(|row| text_width(row) == WIDTH));
    }

    #[test]
    fn every_pixel_row_is_as_wide_as_the_mark() {
        assert!(PIXELS.iter().all(|row| row.chars().count() == WIDTH));
        assert_eq!(PIXELS.len() % 2, 0, "pixels pair up into cells");
    }

    #[test]
    fn the_mark_is_drawn_in_three_of_the_theme_colours() {
        for theme in [Theme::dark(), Theme::light()] {
            let mut colors: Vec<Color> = rows(&theme)
                .iter()
                .flatten()
                .filter_map(|span| span.style.fg)
                .collect();
            colors.sort_by_key(|color| format!("{color:?}"));
            colors.dedup();
            assert_eq!(colors.len(), 3, "{}: {colors:?}", theme.name);
            for color in [theme.accent, theme.border, theme.md_heading] {
                assert!(colors.contains(&color), "{}: {color:?}", theme.name);
            }
        }
    }

    #[test]
    fn two_colours_share_a_cell_as_foreground_over_background() {
        let accent = Some(Color::Red);
        let border = Some(Color::Blue);
        assert_eq!(cell(accent, accent).content, "█");
        let split = cell(accent, border);
        assert_eq!(split.content, "▀");
        assert_eq!(split.style.fg, accent);
        assert_eq!(split.style.bg, border);
        assert_eq!(cell(None, border).content, "▄");
        assert_eq!(cell(None, None).content, " ");
    }
}
