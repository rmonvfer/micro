//! OSC 8 hyperlinks, applied after the frame is laid out.
//!
//! A terminal makes text clickable when it is wrapped in `ESC]8;;url BEL … ESC]8;; BEL`.
//! Those sequences occupy no columns, which is exactly why they cannot be put in a span:
//! everything upstream measures a span by its characters, so a URL inside one would push
//! the rest of the line sideways and break every wrap calculation on it.
//!
//! They can go in afterwards. Ratatui writes each cell's symbol to the terminal verbatim,
//! and a cell owns one column no matter how many bytes its symbol holds — so once layout is
//! settled, the opener can be prepended to the first cell of a link and the terminator
//! appended to its last, with no effect on width at all.
//!
//! Which cells those are is carried through layout by the span's underline colour, set to
//! an index into a table of URLs. Nothing else in the interface sets that field, and it is
//! cleared here before the frame is written, so it never reaches the terminal.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;

/// URLs for one frame, in the order the renderer met them.
///
/// A collector that has been told the terminal cannot do hyperlinks records nothing and
/// marks nothing, so the text falls back to the plain `text (url)` form rather than having
/// its URL swallowed by a terminal that does not understand the escape.
#[derive(Debug, Clone)]
pub struct Links {
    urls: Vec<String>,
    enabled: bool,
}

impl Default for Links {
    fn default() -> Self {
        Links {
            urls: Vec::new(),
            enabled: true,
        }
    }
}

impl Links {
    pub fn new() -> Self {
        Links::default()
    }

    /// Whether the text itself can be made clickable. When it cannot, a caller prints the
    /// target alongside the text instead, since otherwise it would be lost.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// A collector for a terminal that cannot make text clickable.
    pub fn disabled() -> Self {
        Links {
            urls: Vec::new(),
            enabled: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.urls.is_empty()
    }

    /// Record a URL and hand back the style that marks the text pointing at it.
    ///
    /// The marker rides in `underline_color`, which survives being split across wrapped
    /// rows and copied between buffers — so a link broken over two lines is still a link on
    /// both of them.
    pub fn mark(&mut self, style: Style, url: impl Into<String>) -> Style {
        if !self.enabled {
            return style;
        }
        let index = self.urls.len();
        self.urls.push(url.into());
        style.underline_color(sentinel(index))
    }

    /// How many links have been recorded, which is the number the next one will be given.
    pub fn len(&self) -> usize {
        self.urls.len()
    }

    /// Forget every link recorded after `kept`.
    ///
    /// A link's number is written into the rows that point at it, so throwing rows away
    /// means throwing away the links they carried — and keeping the ones before them, whose
    /// numbers are still written into rows that are staying.
    pub fn truncate(&mut self, kept: usize) {
        self.urls.truncate(kept);
    }

    pub fn url(&self, index: usize) -> Option<&str> {
        self.urls.get(index).map(String::as_str)
    }

    /// Wrap every marked run in the buffer with its escape sequences.
    ///
    /// Runs are found per row, so a link split across two rows becomes two hyperlinks
    /// pointing at the same place — which is what a terminal expects, and what makes both
    /// halves clickable.
    pub fn apply(&self, buffer: &mut Buffer, area: Rect) {
        if self.is_empty() {
            return;
        }
        for y in area.top()..area.bottom() {
            let mut x = area.left();
            while x < area.right() {
                let Some(index) = marked_index(buffer, x, y) else {
                    x += 1;
                    continue;
                };
                let mut end = x;
                while end + 1 < area.right() && marked_index(buffer, end + 1, y) == Some(index) {
                    end += 1;
                }
                if let Some(url) = self.url(index) {
                    open(buffer, x, y, url);
                    close(buffer, end, y);
                }
                // The marker has done its work and must not reach the terminal.
                for column in x..=end {
                    clear(buffer, column, y);
                }
                x = end + 1;
            }
        }
    }
}

/// Links are numbered from a base far above any palette index a theme would use, so a
/// marked cell cannot be confused with one a user genuinely underlined in colour.
const BASE: u8 = 16;

fn sentinel(index: usize) -> Color {
    Color::Indexed(BASE.saturating_add(index.min(u8::MAX as usize - BASE as usize) as u8))
}

fn marked_index(buffer: &Buffer, x: u16, y: u16) -> Option<usize> {
    match buffer[(x, y)].underline_color {
        Color::Indexed(value) if value >= BASE => Some((value - BASE) as usize),
        _ => None,
    }
}

fn open(buffer: &mut Buffer, x: u16, y: u16, url: &str) {
    let cell = &mut buffer[(x, y)];
    let symbol = format!("\x1b]8;;{url}\x07{}", cell.symbol());
    cell.set_symbol(&symbol);
}

fn close(buffer: &mut Buffer, x: u16, y: u16) {
    let cell = &mut buffer[(x, y)];
    let symbol = format!("{}\x1b]8;;\x07", cell.symbol());
    cell.set_symbol(&symbol);
}

fn clear(buffer: &mut Buffer, x: u16, y: u16) {
    buffer[(x, y)].underline_color = Color::Reset;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;
    use ratatui::text::Span;

    fn buffer_with(text: &str, style: Style, width: u16) -> (Buffer, Rect) {
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        buffer.set_line(0, 0, &Line::from(vec![Span::styled(text.to_string(), style)]), width);
        (buffer, area)
    }

    #[test]
    fn a_marked_run_is_wrapped_in_its_escapes() {
        let mut links = Links::new();
        let style = links.mark(Style::new(), "https://example.com");
        let (mut buffer, area) = buffer_with("link", style, 10);

        links.apply(&mut buffer, area);

        assert!(buffer[(0, 0)].symbol().starts_with("\x1b]8;;https://example.com\x07"));
        assert!(buffer[(0, 0)].symbol().ends_with('l'));
        assert!(buffer[(3, 0)].symbol().ends_with("\x1b]8;;\x07"));
    }

    /// The cells in between are untouched, so the text still reads as itself.
    #[test]
    fn the_middle_of_a_link_is_left_alone() {
        let mut links = Links::new();
        let style = links.mark(Style::new(), "https://example.com");
        let (mut buffer, area) = buffer_with("link", style, 10);

        links.apply(&mut buffer, area);
        assert_eq!(buffer[(1, 0)].symbol(), "i");
        assert_eq!(buffer[(2, 0)].symbol(), "n");
    }

    #[test]
    fn the_marker_never_reaches_the_terminal() {
        let mut links = Links::new();
        let style = links.mark(Style::new(), "https://example.com");
        let (mut buffer, area) = buffer_with("link", style, 10);

        links.apply(&mut buffer, area);
        for x in 0..4 {
            assert_eq!(buffer[(x, 0)].underline_color, Color::Reset);
        }
    }

    /// On a terminal that cannot do hyperlinks nothing is marked, so nothing is wrapped and
    /// the URL stays visible as text rather than vanishing into a swallowed escape.
    #[test]
    fn a_terminal_without_hyperlinks_gets_none() {
        let mut links = Links::disabled();
        let style = links.mark(Style::new(), "https://example.com");
        assert_eq!(style, Style::new(), "the style is untouched");

        let (mut buffer, area) = buffer_with("link", style, 10);
        links.apply(&mut buffer, area);
        assert_eq!(buffer[(0, 0)].symbol(), "l");
    }

    #[test]
    fn unmarked_text_is_not_touched() {
        let links = Links::new();
        let (mut buffer, area) = buffer_with("plain", Style::new(), 10);
        links.apply(&mut buffer, area);
        assert_eq!(buffer[(0, 0)].symbol(), "p");
    }

    #[test]
    fn two_links_on_one_row_keep_their_own_targets() {
        let mut links = Links::new();
        let first = links.mark(Style::new(), "https://one.example");
        let second = links.mark(Style::new(), "https://two.example");

        let area = Rect::new(0, 0, 12, 1);
        let mut buffer = Buffer::empty(area);
        buffer.set_line(
            0,
            0,
            &Line::from(vec![
                Span::styled("aa", first),
                Span::raw(" "),
                Span::styled("bb", second),
            ]),
            12,
        );

        links.apply(&mut buffer, area);
        assert!(buffer[(0, 0)].symbol().contains("one.example"));
        assert!(buffer[(3, 0)].symbol().contains("two.example"));
    }

    /// The whole path: markdown in, escapes on the terminal's own cells out.
    #[test]
    fn a_markdown_link_becomes_a_hyperlink_on_the_frame() {
        let theme = crate::theme::Theme::dark();
        let mut links = Links::new();
        let blocks = crate::markdown::render_linked(
            "see [the docs](https://example.com) now",
            &theme,
            60,
            &mut links,
            crate::commands::Mermaid::Streaming,
        );

        let area = Rect::new(0, 0, 60, 1);
        let mut buffer = Buffer::empty(area);
        buffer.set_line(0, 0, &Line::from(blocks[0].spans.clone()), 60);
        links.apply(&mut buffer, area);

        let row: String = (0..60).map(|x| buffer[(x, 0)].symbol().to_string()).collect();
        assert!(row.contains("\x1b]8;;https://example.com\x07"), "{row:?}");
        assert!(row.contains("\x1b]8;;\x07"), "and it is closed again");
        // The visible text is untouched: the escapes ride on cells, not in the text.
        let visible: String = row.replace("\x1b]8;;https://example.com\x07", "").replace("\x1b]8;;\x07", "");
        // With the terminal able to click the text, the target rides in the escape rather
        // than being printed after it.
        assert!(visible.starts_with("see the docs now"), "{visible:?}");
    }

    #[test]
    fn a_link_index_survives_the_round_trip() {
        let mut links = Links::new();
        links.mark(Style::new(), "https://a");
        let style = links.mark(Style::new(), "https://b");
        let (buffer, _) = buffer_with("x", style, 4);
        assert_eq!(marked_index(&buffer, 0, 0), Some(1));
        assert_eq!(links.url(1), Some("https://b"));
    }
}
