//! OSC 8 hyperlinks, applied after the frame is laid out.

use ratatui::buffer::Buffer;
use ratatui::buffer::CellDiffOption;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use std::num::NonZeroU16;
use unicode_width::UnicodeWidthStr;

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
                for column in x..=end {
                    if let Some(url) = self.url(index) {
                        link(buffer, column, y, url);
                    }
                    clear(buffer, column, y);
                }
                x = end + 1;
            }
        }
    }
}

/// A `file://` URL for a path a tool was given, resolved against the workspace the way the tool
/// resolved it.
pub fn file_url(path: &str, workspace: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let absolute = match (path.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ => workspace.join(path),
    };

    let mut url = String::from("file://");
    for byte in absolute.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                url.push(byte as char)
            }
            other => url.push_str(&format!("%{other:02X}")),
        }
    }
    url
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

/// Make one cell a link on its own, opening and closing the link around its text. A frame only
/// rewrites the cells that changed, so a link opened in one cell and closed in another could be
/// left open when only one of them is redrawn, turning everything written after it into the link.
/// The escapes take no columns on screen, so the cell keeps the width of its text for that diff.
/// Control characters are dropped from the URL, since one could end the escape early and write
/// whatever follows to the terminal as commands.
fn link(buffer: &mut Buffer, x: u16, y: u16, url: &str) {
    let cell = &mut buffer[(x, y)];
    let Some(width) = NonZeroU16::new(cell.symbol().width() as u16) else {
        return;
    };
    let url: String = url.chars().filter(|c| !c.is_control()).collect();
    let symbol = format!("\x1b]8;;{url}\x07{}\x1b]8;;\x07", cell.symbol());
    cell.set_symbol(&symbol);
    cell.set_diff_option(CellDiffOption::ForcedWidth(width));
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
    fn every_cell_of_a_marked_run_is_a_link_on_its_own() {
        let mut links = Links::new();
        let style = links.mark(Style::new(), "https://example.com");
        let (mut buffer, area) = buffer_with("link", style, 10);

        links.apply(&mut buffer, area);

        for (x, letter) in "link".chars().enumerate() {
            assert_eq!(
                buffer[(x as u16, 0)].symbol(),
                format!("\x1b]8;;https://example.com\x07{letter}\x1b]8;;\x07")
            );
        }
        assert_eq!(buffer[(4, 0)].symbol(), " ", "the link ends with its text");
    }

    /// The escapes take no columns, so a frame's diff still visits the cells after a link and a
    /// change beside it reaches the screen.
    #[test]
    fn the_cells_after_a_link_are_still_redrawn() {
        let mut links = Links::new();
        let style = links.mark(
            Style::new(),
            "file:///Users/someone/a/long/path/to/notes.md",
        );
        let area = Rect::new(0, 0, 12, 1);

        let mut before = Buffer::empty(area);
        before.set_line(
            0,
            0,
            &Line::from(vec![Span::styled("ab", style), Span::raw(" old")]),
            12,
        );
        links.apply(&mut before, area);

        let mut after = Buffer::empty(area);
        after.set_line(
            0,
            0,
            &Line::from(vec![Span::styled("ab", style), Span::raw(" new")]),
            12,
        );
        links.apply(&mut after, area);

        let redrawn: Vec<u16> = before.diff(&after).iter().map(|(x, _, _)| *x).collect();
        assert_eq!(redrawn, vec![3, 4, 5], "only the changed word is redrawn");
    }

    /// A URL from the model cannot end the link's escape early and send the terminal commands.
    #[test]
    fn control_characters_never_leave_the_url() {
        let mut links = Links::new();
        let style = links.mark(
            Style::new(),
            "https://x.test/\x07\x1b]52;c;aGk=\x07\u{9b}2J",
        );
        let (mut buffer, area) = buffer_with("x", style, 4);

        links.apply(&mut buffer, area);

        assert_eq!(
            buffer[(0, 0)].symbol(),
            "\x1b]8;;https://x.test/]52;c;aGk=2J\x07x\x1b]8;;\x07"
        );
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
    fn a_relative_path_resolves_against_the_workspace() {
        let workspace = std::path::Path::new("/work/repo");
        assert_eq!(
            file_url("src/main.rs", workspace),
            "file:///work/repo/src/main.rs"
        );
        assert_eq!(file_url("/etc/hosts", workspace), "file:///etc/hosts");
    }

    #[test]
    fn a_path_is_percent_encoded_where_a_url_needs_it() {
        let workspace = std::path::Path::new("/work");
        assert_eq!(
            file_url("my notes#1?.md", workspace),
            "file:///work/my%20notes%231%3F.md"
        );
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
