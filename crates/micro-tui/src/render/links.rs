//! OSC 8 hyperlinks, applied after the frame is laid out.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;

/// URLs for one frame, in the order the renderer met them.
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

    /// Whether the text itself can be made clickable.
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
    pub fn truncate(&mut self, kept: usize) {
        self.urls.truncate(kept);
    }

    pub fn url(&self, index: usize) -> Option<&str> {
        self.urls.get(index).map(String::as_str)
    }

    /// Wrap every marked run in the buffer with its escape sequences.
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

                for column in x..=end {
                    clear(buffer, column, y);
                }
                x = end + 1;
            }
        }
    }
}

/// Links are numbered from a base far above any palette index a theme would use.
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
        buffer.set_line(
            0,
            0,
            &Line::from(vec![Span::styled(text.to_string(), style)]),
            width,
        );
        (buffer, area)
    }

    #[test]
    fn a_marked_run_is_wrapped_in_its_escapes() {
        let mut links = Links::new();
        let style = links.mark(Style::new(), "https://example.com");
        let (mut buffer, area) = buffer_with("link", style, 10);

        links.apply(&mut buffer, area);

        assert!(buffer[(0, 0)]
            .symbol()
            .starts_with("\x1b]8;;https://example.com\x07"));
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

    /// On a terminal that cannot do hyperlinks nothing is marked.
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

        let row: String = (0..60)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();
        assert!(row.contains("\x1b]8;;https://example.com\x07"), "{row:?}");
        assert!(row.contains("\x1b]8;;\x07"), "and it is closed again");

        let visible: String = row
            .replace("\x1b]8;;https://example.com\x07", "")
            .replace("\x1b]8;;\x07", "");

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
