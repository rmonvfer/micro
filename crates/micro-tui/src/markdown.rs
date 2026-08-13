//! Just enough markdown for a terminal, painted the way ohm paints it.
//!
//! Model output is markdown, and reading it raw is worse than reading nothing. This covers
//! what actually shows up in an answer — fenced code, headings, lists, quotes, links, rules
//! and inline emphasis — and leaves anything more elaborate as plain text rather than
//! guessing.
//!
//! Every element takes the token ohm gives it, so the two interfaces agree on what colors a
//! heading against what colors the fence around a code block. ohm parses markdown into a
//! token tree and renders that; this reads the source a line at a time, which is what keeps
//! a half-streamed response legible. Where that difference shows, it is noted at the point
//! it matters.

pub mod syntax;

use crate::render::links::Links;
use crate::theme::Theme;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use syntax::Highlighter;

/// How far a fenced block's contents are indented, matching ohm's `codeBlockIndent`.
const CODE_INDENT: &str = "  ";

/// Widest a horizontal rule is drawn, however wide the terminal is. ohm's own cap.
const MAX_RULE: usize = 80;

/// Widest a single table column grows before its cells start losing characters. Without a
/// cap one verbose column would starve every other column in a narrow terminal.
const MAX_COLUMN: usize = 24;

/// A table narrower than this is not rendered as a table, because nothing but its border
/// would be left; the source lines are shown as they arrived instead.
const MIN_TABLE_WIDTH: usize = 8;

/// One source line, styled and ready to wrap.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub spans: Vec<Span<'static>>,
    /// Columns to indent continuation rows by when this line wraps.
    pub indent: usize,
    /// Whether the line's background extends to the full width. ohm paints no background
    /// behind code, so nothing sets this now; it is kept for a caller that still reads it.
    pub filled: bool,
    /// Whether a blank row belongs after this block when the source does not already have
    /// one — true for a heading, a quote and a rule, which are set apart from what follows.
    pub spaced_after: bool,
}

impl Block {
    fn spaced(mut self) -> Self {
        self.spaced_after = true;
        self
    }

    fn plain(spans: Vec<Span<'static>>) -> Self {
        Block {
            spans,
            indent: 0,
            filled: false,
            spaced_after: false,
        }
    }
}

/// Style `text` line by line. `width` is the column budget a horizontal rule spans.
/// Render markdown, recording every link so the frame can make them clickable.
///
/// The collector is what carries a URL from here to the terminal: link text is marked as it
/// is styled, and the escapes go on after layout, where they cost no columns.
pub fn render_linked(
    text: &str,
    theme: &Theme,
    width: usize,
    links: &mut Links,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut fence: Option<Fence> = None;
    // Rows accumulate until the table ends, because how wide a column should be is only
    // known once every row has been read.
    let mut table: Option<Table> = None;
    // Display maths, gathering until its closing delimiter: it is set over several rows, so
    // nothing of it can be drawn until all of it has arrived.
    let mut maths: Option<Maths> = None;
    // Where a numbered list has got to, so it counts on rather than echoing the source.
    let mut ordinal: Option<usize> = None;

    let lines: Vec<&str> = text.split('\n').collect();
    for (index, line) in lines.iter().enumerate() {
        let line = *line;
        let trimmed = line.trim_start();
        // Whether the source already leaves a gap after this line, which is what decides if
        // one has to be made.
        let next = lines.get(index + 1).map(|next| next.trim_start());
        let followed_by_blank = next.is_none_or(|next| next.is_empty());
        // A run of quote lines is one block, so only the last of them is set apart.
        let continues = next.is_some_and(|next| {
            quote(trimmed).is_some() && quote(next).is_some()
        });

        if let Some(open) = &mut maths {
            match trimmed.starts_with(open.closer) {
                true => {
                    blocks.extend(open.drawn(theme));
                    maths = None;
                }
                false => open.lines.push(line.to_string()),
            }
            continue;
        }
        if let Some(open) = opens_maths(trimmed) {
            maths = Some(open);
            continue;
        }

        match &mut fence {
            // A fence closes on its own marker, so a `~~~` inside a ``` block is content.
            Some(open) if trimmed.starts_with(open.marker) => {
                blocks.push(Block::plain(vec![Span::styled(
                    trimmed.to_string(),
                    Style::new().fg(theme.md_code_block_border),
                )]));
                fence = None;
                if !followed_by_blank {
                    blocks.push(Block::plain(Vec::new()));
                }
            }
            Some(open) => blocks.push(code_line(line, open, theme)),
            None => match fence_marker(trimmed) {
                Some(marker) => {
                    // A table cannot run into a code block, so it is finished here rather
                    // than left open to be drawn after the block it precedes.
                    if let Some(open) = table.take() {
                        blocks.extend(open.render(theme, width, links));
                    }
                    // The fence line is shown with its language, as ohm shows it, rather
                    // than being swallowed.
                    blocks.push(Block::plain(vec![Span::styled(
                        trimmed.to_string(),
                        Style::new().fg(theme.md_code_block_border),
                    )]));
                    let language = &trimmed[marker.len()..];
                    fence = Some(Fence {
                        marker,
                        highlighter: Highlighter::new(language),
                    });
                }
                None => {
                    // A table swallows the lines that belong to it and is drawn once, at
                    // the line that ends it: a column's width is not known before then.
                    match table.take() {
                        Some(open) => match open.take_line(trimmed) {
                            Continued::Taken(open) => table = Some(open),
                            Continued::Ended(open) => {
                                blocks.extend(open.render(theme, width, links));
                                let block =
                                    block_for(line, trimmed, theme, width, links, &mut ordinal);
                                let spaced = block.spaced_after;
                                blocks.push(block);
                                if spaced && !followed_by_blank && !continues {
                                    blocks.push(Block::plain(Vec::new()));
                                }
                            }
                        },
                        None => match starts_table(trimmed) {
                            Some(started) => table = Some(started),
                            None => {
                                let block =
                                    block_for(line, trimmed, theme, width, links, &mut ordinal);
                                let spaced = block.spaced_after;
                                blocks.push(block);
                                // A heading, a quote or a rule is followed by a blank row,
                                // which is what sets it apart from the paragraph after it.
                                if spaced && !followed_by_blank && !continues {
                                    blocks.push(Block::plain(Vec::new()));
                                }
                            }
                        },
                    }
                }
            },
        }
    }

    // Maths the text ends in the middle of is drawn with what it has, so a half-streamed
    // answer shows the expression rather than the source it was written in.
    if let Some(open) = maths {
        blocks.extend(open.drawn(theme));
    }

    // A table the text ends in the middle of is drawn with what it has, so a half-streamed
    // answer shows its rows rather than nothing.
    if let Some(open) = table {
        blocks.extend(open.render(theme, width, links));
    }

    // A fence the text never closed is closed here, so a half-written answer still reads as
    // a code block rather than running on into whatever follows it.
    if let Some(open) = fence {
        blocks.push(Block::plain(vec![Span::styled(
            open.marker.to_string(),
            Style::new().fg(theme.md_code_block_border),
        )]));
    }

    blocks
}

/// An open fence: the marker that closes it, and the highlighter for its language when the
/// language is one micro knows.
struct Fence {
    marker: &'static str,
    highlighter: Option<Highlighter>,
}

/// One line inside a fence.
///
/// A language micro can lex is painted token by token; anything else keeps the block's own
/// color, byte for byte as it arrived. ohm makes the same split, and declines to guess at an
/// untagged block for the same reason: guessing reads prose as code.
fn code_line(line: &str, fence: &mut Fence, theme: &Theme) -> Block {
    let plain = Style::new().fg(theme.md_code_block);
    let mut spans = vec![Span::styled(CODE_INDENT, plain)];

    match &mut fence.highlighter {
        Some(highlighter) => spans.extend(highlighter.line(line).into_iter().map(|token| {
            let style = token.scope.map_or(plain, |scope| scope.style(theme));
            Span::styled(token.text, style)
        })),
        None => spans.push(Span::styled(line.to_string(), plain)),
    }

    Block {
        spans,
        indent: CODE_INDENT.len(),
        filled: false,
        spaced_after: false,
    }
}

/// The fence marker a line opens, if it opens one.
fn fence_marker(trimmed: &str) -> Option<&'static str> {
    ["```", "~~~"]
        .into_iter()
        .find(|marker| trimmed.starts_with(marker))
}

fn block_for(
    line: &str,
    trimmed: &str,
    theme: &Theme,
    width: usize,
    links: &mut Links,
    ordinal: &mut Option<usize>,
) -> Block {
    if is_rule(trimmed) {
        return Block::plain(vec![Span::styled(
            "─".repeat(width.clamp(1, MAX_RULE)),
            Style::new().fg(theme.md_hr),
        )])
        .spaced();
    }

    if let Some((level, rest)) = heading(trimmed) {
        // ohm keeps the hashes only from the third level down, where they are the only thing
        // left to tell one heading from another.
        let style = heading_style(level, theme);
        let mut spans = Vec::new();
        if level >= 3 {
            spans.push(Span::styled(format!("{} ", "#".repeat(level)), style));
        }
        spans.extend(inline(rest, style, theme, links));
        return Block::plain(spans).spaced();
    }

    if let Some(rest) = quote(trimmed) {
        let mut spans = vec![Span::styled("│ ", Style::new().fg(theme.md_quote_border))];
        let body = Style::new()
            .fg(theme.md_quote)
            .add_modifier(Modifier::ITALIC);
        spans.extend(inline(rest, body, theme, links));
        return Block {
            spans,
            indent: 2,
            filled: false,
            spaced_after: true,
        };
    }

    if let Some((marker, rest)) = bullet(line, ordinal) {
        let indent = crate::wrap::text_width(&marker);
        let mut spans = vec![Span::styled(marker, Style::new().fg(theme.md_list_bullet))];
        spans.extend(inline(rest, theme.body(), theme, links));
        return Block {
            spans,
            indent,
            filled: false,
            spaced_after: false,
        };
    }

    Block::plain(inline(line, theme.body(), theme, links))
}

/// ohm underlines a top-level heading and bolds every level.
fn heading_style(level: usize, theme: &Theme) -> Style {
    let style = Style::new()
        .fg(theme.md_heading)
        .add_modifier(Modifier::BOLD);
    if level == 1 {
        style.add_modifier(Modifier::UNDERLINED)
    } else {
        style
    }
}

fn is_rule(trimmed: &str) -> bool {
    trimmed.len() >= 3
        && (trimmed.chars().all(|c| c == '-')
            || trimmed.chars().all(|c| c == '*')
            || trimmed.chars().all(|c| c == '_'))
}

fn heading(trimmed: &str) -> Option<(usize, &str)> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        trimmed[hashes..]
            .strip_prefix(' ')
            .map(|rest| (hashes, rest))
    } else {
        None
    }
}

/// The text of a quote line. A bare `>` opens an empty quote line rather than nothing.
fn quote(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// A list marker and the text after it.
///
/// ohm normalizes an unordered marker to `- ` and an ordered one to its own number, and
/// indents each level by four columns. Source indentation is mapped onto that grid so a
/// nested item lines up the way ohm nests it rather than however the model happened to
/// space it.
fn bullet<'a>(line: &'a str, ordinal: &mut Option<usize>) -> Option<(String, &'a str)> {
    let leading = line.len() - line.trim_start().len();
    let trimmed = &line[leading..];
    // Two spaces is the shallowest nesting a model reliably emits, so depth counts by two
    // and renders by four.
    let depth = leading / 2;
    let padding = " ".repeat(depth * 4);

    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            *ordinal = None;
            let (task, rest) = task_marker(rest);
            return Some((format!("{padding}- {task}"), rest));
        }
    }

    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let after = &trimmed[digits..];
        for marker in [". ", ") "] {
            if let Some(rest) = after.strip_prefix(marker) {
                let (task, rest) = task_marker(rest);
                // A numbered list counts on from where it started rather than repeating
                // whatever the source wrote, so `1. 2. 10.` reads `1. 2. 3.`.
                let number = match ordinal {
                    Some(previous) => {
                        *previous += 1;
                        *previous
                    }
                    None => {
                        let start = trimmed[..digits].parse().unwrap_or(1);
                        *ordinal = Some(start);
                        start
                    }
                };
                return Some((format!("{padding}{number}{marker}{task}"), rest));
            }
        }
    }

    None
}

/// A task list checkbox, which ohm renders as part of the marker rather than the text.
fn task_marker(rest: &str) -> (String, &str) {
    for (source, rendered) in [("[ ] ", "[ ] "), ("[x] ", "[x] "), ("[X] ", "[x] ")] {
        if let Some(after) = rest.strip_prefix(source) {
            return (rendered.to_string(), after);
        }
    }
    (String::new(), rest)
}

/// Apply inline emphasis. An unterminated marker stays literal rather than swallowing the
/// rest of the line, which matters while a response is still streaming in.
fn inline(text: &str, base: Style, theme: &Theme, links: &mut Links) -> Vec<Span<'static>> {
    let characters: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buffer = String::new();
    let mut index = 0;

    while index < characters.len() {
        if characters[index] == '<' {
            if let Some((rendered, next)) = autolink(&characters, index, theme, links) {
                if !buffer.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buffer), base));
                }
                spans.extend(rendered);
                index = next;
                continue;
            }
        }

        // Maths reads as maths only once it is drawn: `\alpha` is a word, α is a letter.
        if matches!(characters[index], '$' | '\\') {
            if let Some((rendered, next)) = math(&characters, index) {
                if !buffer.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buffer), base));
                }
                spans.push(Span::styled(rendered, base));
                index = next;
                continue;
            }
        }

        if characters[index] == '[' {
            if let Some((rendered, next)) = link(&characters, index, base, theme, links) {
                if !buffer.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buffer), base));
                }
                spans.extend(rendered);
                index = next;
                continue;
            }
        }

        let matched = match characters[index] {
            // ohm's inline code is a color, not a tint; no background is painted behind it.
            '`' => marker(&characters, index, "`")
                .map(|(content, next)| (content, next, Style::new().fg(theme.md_code))),
            '*' if characters.get(index + 1) == Some(&'*') => marker(&characters, index, "**")
                .map(|(content, next)| (content, next, base.add_modifier(Modifier::BOLD))),
            '_' if characters.get(index + 1) == Some(&'_') => marker(&characters, index, "__")
                .map(|(content, next)| (content, next, base.add_modifier(Modifier::BOLD))),
            '~' if characters.get(index + 1) == Some(&'~') => marker(&characters, index, "~~")
                .map(|(content, next)| {
                    (content, next, base.add_modifier(Modifier::CROSSED_OUT))
                }),
            // Single markers are emphasis. Checked after the doubled ones, so `**bold**` is
            // never read as an empty italic wrapping a bold.
            '*' => marker(&characters, index, "*")
                .map(|(content, next)| (content, next, base.add_modifier(Modifier::ITALIC))),
            '_' => marker(&characters, index, "_")
                .map(|(content, next)| (content, next, base.add_modifier(Modifier::ITALIC))),
            _ => None,
        };

        match matched {
            Some((content, next, style)) => {
                if !buffer.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buffer), base));
                }
                spans.push(Span::styled(content, style));
                index = next;
            }
            None => {
                buffer.push(characters[index]);
                index += 1;
            }
        }
    }

    if !buffer.is_empty() || spans.is_empty() {
        spans.push(Span::styled(buffer, base));
    }
    spans
}

/// An `<https://example.com>` autolink, whose text is its own target.
///
/// The angle brackets are the whole syntax, so what is shown is the URL itself and there is
/// nothing to print after it — a link that already is its target does not say it twice.
fn autolink(
    characters: &[char],
    start: usize,
    theme: &Theme,
    links: &mut Links,
) -> Option<(Vec<Span<'static>>, usize)> {
    let end = find(characters, start + 1, '>')?;
    let href: String = characters[start + 1..end].iter().collect();
    // Only a real scheme counts, or every comparison in prose becomes a link.
    if !href.starts_with("http://") && !href.starts_with("https://") && !href.starts_with("mailto:")
    {
        return None;
    }

    let style = links.mark(
        Style::new()
            .fg(theme.md_link)
            .add_modifier(Modifier::UNDERLINED),
        &href,
    );
    Some((vec![Span::styled(href, style)], end + 1))
}

/// A `[text](href)` link starting at `start`, and the index past its closing paren.
///
/// ohm underlines the text and prints the target after it when the two differ, so a link
/// whose text already is its target does not say it twice.
fn link(
    characters: &[char],
    start: usize,
    base: Style,
    theme: &Theme,
    links: &mut Links,
) -> Option<(Vec<Span<'static>>, usize)> {
    let close = find(characters, start + 1, ']')?;
    if characters.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = find(characters, close + 2, ')')?;

    let text: String = characters[start + 1..close].iter().collect();
    let href: String = characters[close + 2..end].iter().collect();
    if text.is_empty() || href.is_empty() {
        return None;
    }

    // Marked here so the frame can wrap exactly this text in its escapes once the line has
    // been laid out and every column is settled.
    let style = links.mark(
        Style::new()
            .fg(theme.md_link)
            .add_modifier(Modifier::UNDERLINED),
        &href,
    );
    let mut spans = inline(&text, style, theme, links);
    // An autolinked address arrives as text without the scheme the href carries.
    let bare = href.strip_prefix("mailto:").unwrap_or(&href);
    // Where the terminal can make the text itself clickable the target is already carried,
    // and printing it again would say the same thing twice.
    if !links.is_enabled() && text != href && text != bare {
        spans.push(Span::styled(
            format!(" ({href})"),
            Style::new().fg(theme.md_link_url),
        ));
    }
    let _ = base;
    Some((spans, end + 1))
}

fn find(characters: &[char], from: usize, wanted: char) -> Option<usize> {
    (from..characters.len()).find(|index| characters[*index] == wanted)
}

/// Display maths in progress: what closes it, and the source gathered so far.
struct Maths {
    closer: &'static str,
    lines: Vec<String>,
}

/// Whether a line opens display maths, and what will close it.
///
/// `$$` and `\[` on a line of their own. Written on the same line as the expression they
/// open, the expression is the rest of that line.
fn opens_maths(trimmed: &str) -> Option<Maths> {
    let (opener, closer) = match trimmed {
        line if line.starts_with("$$") => ("$$", "$$"),
        line if line.starts_with("\\[") => ("\\[", "\\]"),
        _ => return None,
    };
    let rest = trimmed[opener.len()..].trim();
    // A closer on the same line makes it one line of maths rather than a block to gather.
    if rest.ends_with(closer) && !rest.is_empty() {
        return Some(Maths {
            closer,
            lines: vec![rest[..rest.len() - closer.len()].to_string()],
        });
    }
    Some(Maths {
        closer,
        lines: match rest.is_empty() {
            true => Vec::new(),
            false => vec![rest.to_string()],
        },
    })
}

impl Maths {
    /// The expression, set out over as many rows as it needs.
    ///
    /// What cannot be drawn is shown as it was written: an expression micro does not
    /// understand is still something the reader asked to see.
    fn drawn(&self, theme: &Theme) -> Vec<Block> {
        let source = self.lines.join("\n");
        let style = theme.body();
        let drawn = crate::latex::render_display(&source).unwrap_or_else(|| source.clone());
        let mut blocks = vec![Block::plain(Vec::new())];
        for line in drawn.split('\n') {
            blocks.push(Block::plain(vec![Span::styled(line.to_string(), style)]));
        }
        blocks.push(Block::plain(Vec::new()));
        blocks
    }
}

/// A table in progress. Rows accumulate until the table ends, because a column's width is
/// only known once every row has been read.
enum Table {
    /// A line that contains a pipe but whose delimiter has not arrived yet. It may be a
    /// header, or it may be prose with a pipe in it — the next line decides.
    Pending { header: String },
    /// A header whose delimiter confirmed it, and every row read since. The first entry in
    /// `cells` is the header row.
    Building {
        cells: Vec<Vec<String>>,
        alignments: Vec<Align>,
    },
}

/// The alignment a delimiter column declares, as ohm reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
}

/// The start of a table: a line that holds more than one cell. Anything further is decided
/// once the next line either delivers a delimiter or does not.
fn starts_table(trimmed: &str) -> Option<Table> {
    contains_cell(trimmed).then(|| Table::Pending {
        header: trimmed.to_string(),
    })
}

/// Whether a line holds more than one cell. A pipe on its own in prose — `a | b` — is not
/// a table; ohm makes the same demand of two cells before committing.
fn contains_cell(trimmed: &str) -> bool {
    trimmed.contains('|') && split_cells(trimmed).len() >= 2
}

/// The cells of a table row, without the outer pipes. Interior pipes split; spaces around
/// a cell are the source's own spacing and are not part of the value.
fn split_cells(trimmed: &str) -> Vec<String> {
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').map(|cell| cell.trim().to_string()).collect()
}

/// The delimiter row that confirms a table, as the alignment of each of its columns. A run
/// of dashes alone, with no pipe, is a rule and is left to the rule renderer.
fn delimiter(trimmed: &str) -> Option<Vec<Align>> {
    let cells = split_cells(trimmed);
    let mut alignments = Vec::with_capacity(cells.len());
    for cell in &cells {
        let dashes = cell.trim_matches(':');
        if dashes.len() < 3 || !dashes.chars().all(|c| c == '-') {
            return None;
        }
        alignments.push(match (cell.starts_with(':'), cell.ends_with(':')) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        });
    }
    (!alignments.is_empty()).then_some(alignments)
}

/// What became of a line offered to a table.
enum Continued {
    /// The line belonged to the table, which carries on.
    Taken(Table),
    /// The line was not part of it, so the table is finished and the line is still to be
    /// dealt with.
    Ended(Table),
}

impl Table {
    /// Offer the next line to the table.
    ///
    /// A header waits for a delimiter naming as many columns as it has; anything else means
    /// the pipe was prose. Once building, a line goes on taking rows until one arrives with
    /// no cells in it.
    fn take_line(self, trimmed: &str) -> Continued {
        match self {
            Table::Pending { header } => {
                let header_cells = split_cells(&header);
                match delimiter(trimmed).filter(|widths| widths.len() == header_cells.len()) {
                    Some(alignments) => Continued::Taken(Table::Building {
                        cells: vec![header_cells],
                        alignments,
                    }),
                    None => Continued::Ended(Table::Pending { header }),
                }
            }
            Table::Building { mut cells, alignments } => match contains_cell(trimmed) {
                true => {
                    cells.push(split_cells(trimmed));
                    Continued::Taken(Table::Building { cells, alignments })
                }
                false => Continued::Ended(Table::Building { cells, alignments }),
            },
        }
    }

    /// Draw the table as one block per row, in the space `width` leaves for it.
    ///
    /// Columns are sized to their widest cell, up to a cap, and the widest columns are
    /// shortened with an ellipsis until the whole row fits. A table too wide to say
    /// anything in is not drawn as a table at all; its rows fall back to the text they
    /// arrived as.
    fn render(self, theme: &Theme, width: usize, links: &mut Links) -> Vec<Block> {
        let Self::Building { cells, alignments } = self else {
            // A pending header never became a table, so it is the text it always was.
            let Table::Pending { header } = self else {
                unreachable!()
            };
            return vec![Block::plain(inline(&header, theme.body(), theme, links))];
        };

        let columns = alignments.len();
        let mut widths: Vec<usize> = (0..columns)
            .map(|column| {
                cells
                    .iter()
                    .map(|row| row.get(column).map_or(0, |cell| crate::wrap::text_width(cell)))
                    .max()
                    .unwrap_or(0)
                    .min(MAX_COLUMN)
            })
            .collect();

        // The widest columns give up columns until the row fits. An empty cell asks for
        // nothing, so the floor of 1 is never the thing being shortened.
        while row_width(&widths) > width {
            match widths.iter().position(|w| *w == *widths.iter().max().unwrap()) {
                Some(widest) if widths[widest] > 1 => widths[widest] -= 1,
                _ => break,
            }
        }

        if width < MIN_TABLE_WIDTH || row_width(&widths) > width {
            return cells
                .into_iter()
                .map(|row| Block::plain(inline(&row.join(" | "), theme.body(), theme, links)))
                .collect();
        }

        let border = Style::new().fg(theme.md_hr);
        let header_style = Style::new()
            .fg(theme.md_heading)
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::UNDERLINED);
        let body = theme.body();

        let mut blocks = Vec::with_capacity(cells.len());
        for (index, row) in cells.iter().enumerate() {
            let style = match index {
                0 => header_style,
                _ => body,
            };
            let mut spans = Vec::new();
            for (column, cell_width) in widths.iter().enumerate() {
                if column > 0 {
                    spans.push(Span::styled(" │ ", border));
                }
                let cell = row.get(column).map_or("", String::as_str);
                let fitted = crate::wrap::truncate(cell, *cell_width);
                let used = crate::wrap::text_width(&fitted);
                let padding = cell_width - used;
                let (before, after) = match alignments[column] {
                    Align::Left => (0, padding),
                    Align::Right => (padding, 0),
                    Align::Center => (padding / 2, padding - padding / 2),
                };
                if before > 0 {
                    spans.push(Span::styled(" ".repeat(before), style));
                }
                spans.extend(inline(&fitted, style, theme, links));
                if after > 0 {
                    spans.push(Span::styled(" ".repeat(after), style));
                }
            }
            blocks.push(Block {
                spans,
                indent: 0,
                filled: false,
                spaced_after: false,
            });
        }
        blocks
    }
}

/// The columns a row spans: every cell, plus the ` │ ` between each pair.
fn row_width(widths: &[usize]) -> usize {
    let cells: usize = widths.iter().sum();
    cells + 3 * widths.len().saturating_sub(1)
}

/// Inline maths, drawn as the characters it stands for, and the index past its closer.
///
/// The delimiters ohm reads: `$...$`, `$$...$$` and `\(...\)`. A lone `$` before a space,
/// or one that never closes on the same line, is a dollar sign and is left alone — a price
/// in the middle of a sentence must not swallow the rest of it.
fn math(characters: &[char], start: usize) -> Option<(String, usize)> {
    let (opener, closer) = match (characters[start], characters.get(start + 1)) {
        ('$', Some('$')) => ("$$", "$$"),
        ('\\', Some('(')) => ("\\(", "\\)"),
        ('$', Some(next)) if !next.is_whitespace() => ("$", "$"),
        _ => return None,
    };

    let (source, next) = marker(characters, start, opener)
        .filter(|(source, _)| !source.is_empty())?;
    // A single `$` is also a currency sign and a shell variable, so it has to earn being
    // read as maths: what follows a price is a digit, what precedes the closing `$` of a
    // sum of money is a space, and `$PATH` is a name rather than an expression.
    if opener == "$" && !is_math(&source, characters.get(next)) {
        return None;
    }
    // `marker` closes on the opener, which is the closer for every delimiter but `\(`.
    let next = match closer == opener {
        true => next,
        false => {
            let text: String = characters[start + opener.len()..].iter().collect();
            let end = text.find(closer)?;
            return crate::latex::render(&text[..end])
                .map(|drawn| (drawn, start + opener.len() + end + closer.len()));
        }
    };
    crate::latex::render(&source).map(|drawn| (drawn, next))
}

/// Whether what a lone `$` fenced is maths rather than money or a variable.
///
/// `$5 and $10` closes on the second dollar with `5 and ` between them: it ends in a space,
/// and a digit follows the closer. Either is enough to say this was never an expression.
fn is_math(source: &str, after: Option<&char>) -> bool {
    if source.ends_with(char::is_whitespace) || source.contains('`') {
        return false;
    }
    if after.is_some_and(char::is_ascii_digit) {
        return false;
    }
    // `$PATH` followed by a word: a name being spelled out, not a product being written.
    let shouted = !source.is_empty()
        && source
            .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_')
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    !(shouted && after.is_some_and(|c| c.is_alphanumeric() || *c == '_'))
}

/// The text delimited by `delimiter` starting at `start`, and the index past the closer.
fn marker(characters: &[char], start: usize, delimiter: &str) -> Option<(String, usize)> {
    let opener: Vec<char> = delimiter.chars().collect();
    let width = opener.len();
    let mut index = start + width;

    while index + width <= characters.len() {
        if characters[index..index + width] == opener[..] {
            let content: String = characters[start + width..index].iter().collect();
            return (!content.is_empty()).then_some((content, index + width));
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    /// A table is drawn as a table: columns sized to what is in them, aligned as the
    /// delimiter asked, with the header set apart.
    #[test]
    fn a_table_is_drawn_as_one() {
        let rows = drawn("| Model | Context |\n| --- | ---: |\n| opus | 200k |\n| gemini | 1M |");
        assert_eq!(rows[0], "Model  \u{2502} Context");
        assert_eq!(rows[1], "opus   \u{2502}    200k", "right-aligned, as the delimiter asked");
        assert_eq!(rows[2], "gemini \u{2502}      1M");
    }

    /// A pipe in prose is a pipe: without a delimiter row beneath it, nothing is a table.
    #[test]
    fn a_pipe_in_prose_is_not_a_table() {
        assert_eq!(drawn("a | b\nand more"), vec!["a | b", "and more"]);
    }

    /// Maths reads as maths only once it is drawn.
    #[test]
    fn inline_maths_is_drawn_as_the_characters_it_stands_for() {
        assert_eq!(
            drawn("The set $\\alpha_1 + \\beta^2$ holds."),
            vec!["The set \u{3b1}\u{2081} + \u{3b2}\u{b2} holds."]
        );
    }

    /// A dollar is money and a shell variable more often than it is maths, so it has to
    /// earn being read as an expression.
    #[test]
    fn a_dollar_sign_is_left_alone() {
        assert_eq!(drawn("costs $5 and $10 here."), vec!["costs $5 and $10 here."]);
        assert_eq!(drawn("set $PATH and $HOME now"), vec!["set $PATH and $HOME now"]);
        assert_eq!(drawn("a lone $ sign"), vec!["a lone $ sign"]);
    }

    /// Every row of a block, as plain text.
    fn drawn(source: &str) -> Vec<String> {
        let mut links = Links::default();
        render_linked(source, &Theme::dark(), 60, &mut links)
            .iter()
            .map(|block| {
                block
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .filter(|row: &String| !row.is_empty())
            .collect()
    }

    use super::*;

    fn theme() -> Theme {
        Theme::dark()
    }

    /// The renderer at a width wide enough that only a horizontal rule notices.
    fn render(text: &str, theme: &Theme) -> Vec<Block> {
        render_linked(text, theme, MAX_RULE, &mut Links::new())
    }

    fn text_of(block: &Block) -> String {
        block
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn rendered(blocks: &[Block]) -> Vec<String> {
        blocks.iter().map(text_of).collect()
    }

    fn span_with<'a>(block: &'a Block, text: &str) -> &'a Span<'static> {
        block
            .spans
            .iter()
            .find(|span| span.content.as_ref() == text)
            .unwrap_or_else(|| {
                panic!(
                    "no span {text:?} in {:?}",
                    rendered(std::slice::from_ref(block))
                )
            })
    }

    #[test]
    fn plain_lines_pass_through() {
        let blocks = render("hello there", &theme());
        assert_eq!(text_of(&blocks[0]), "hello there");
        assert!(!blocks[0].filled);
    }

    #[test]
    fn a_fence_keeps_its_marker_and_language() {
        let theme = theme();
        let blocks = render("before\n```rust\nlet x = 1;\n```\nafter", &theme);
        assert_eq!(
            rendered(&blocks),
            // A closed fence is set apart from what follows it, the way pi sets it apart.
            vec!["before", "```rust", "  let x = 1;", "```", "", "after"]
        );
        assert_eq!(
            blocks[1].spans[0].style.fg,
            Some(theme.md_code_block_border)
        );
        assert_eq!(
            blocks[3].spans[0].style.fg,
            Some(theme.md_code_block_border)
        );
    }

    #[test]
    fn code_is_colored_rather_than_tinted() {
        let theme = theme();
        let blocks = render("```\nlet x = 1;\n```", &theme);
        assert_eq!(blocks[1].spans[0].style.fg, Some(theme.md_code_block));
        // ohm paints no background behind a code block.
        assert_eq!(blocks[1].spans[0].style.bg, None);
        assert!(!blocks[1].filled);
    }

    #[test]
    fn a_fence_closes_only_on_its_own_marker() {
        let blocks = render("```\n~~~ not the closer\n```", &theme());
        assert_eq!(
            rendered(&blocks),
            vec!["```", "  ~~~ not the closer", "```"]
        );
    }

    #[test]
    fn an_unterminated_fence_still_styles_what_followed_it() {
        let theme = theme();
        let blocks = render("```\nstreaming code", &theme);
        assert_eq!(rendered(&blocks), // A fence the answer never closed is closed for it, so it still reads as code.
            vec!["```", "  streaming code", "```"]);
        assert_eq!(blocks[1].spans[0].style.fg, Some(theme.md_code_block));
    }

    #[test]
    fn a_top_level_heading_drops_its_hashes_and_is_underlined() {
        let theme = theme();
        let blocks = render("# Title", &theme);
        assert_eq!(text_of(&blocks[0]), "Title");

        let style = blocks[0].spans[0].style;
        assert_eq!(style.fg, Some(theme.md_heading));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn a_second_level_heading_drops_its_hashes_without_an_underline() {
        let blocks = render("## Title", &theme());
        assert_eq!(text_of(&blocks[0]), "Title");
        let style = blocks[0].spans[0].style;
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(!style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn a_deeper_heading_keeps_its_hashes() {
        assert_eq!(rendered(&render("### Deep", &theme())), vec!["### Deep"]);
        assert_eq!(
            rendered(&render("###### Deepest", &theme())),
            vec!["###### Deepest"]
        );
    }

    #[test]
    fn a_bullet_keeps_ohms_dash_and_nests_by_four() {
        let blocks = render("- first\n  - nested\n    - deeper\n3. third", &theme());
        assert_eq!(
            rendered(&blocks),
            vec!["- first", "    - nested", "        - deeper", "3. third"]
        );
        assert_eq!(blocks[0].indent, 2);
        assert_eq!(blocks[1].indent, 6);
        assert_eq!(blocks[2].indent, 10);
        assert_eq!(blocks[3].indent, 3);
    }

    #[test]
    fn a_bullet_marker_takes_the_list_bullet_color() {
        let theme = theme();
        let blocks = render("- first", &theme);
        assert_eq!(blocks[0].spans[0].style.fg, Some(theme.md_list_bullet));
    }

    #[test]
    fn a_task_marker_belongs_to_the_bullet() {
        let blocks = render("- [ ] open\n- [x] done\n- [X] also done", &theme());
        assert_eq!(
            rendered(&blocks),
            vec!["- [ ] open", "- [x] done", "- [x] also done"]
        );
        assert_eq!(blocks[0].spans[0].content.as_ref(), "- [ ] ");
    }

    #[test]
    fn inline_code_takes_its_own_color_and_no_background() {
        let theme = theme();
        let blocks = render("call `run()` first", &theme);
        let code = span_with(&blocks[0], "run()");
        assert_eq!(code.style.fg, Some(theme.md_code));
        assert_eq!(code.style.bg, None);
    }

    #[test]
    fn bold_survives_alongside_code() {
        let blocks = render("run `cargo test` and **stop**", &theme());
        let styled: Vec<&str> = blocks[0]
            .spans
            .iter()
            .filter(|span| span.style != theme().body())
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(styled, vec!["cargo test", "stop"]);
    }

    #[test]
    fn an_unclosed_marker_stays_literal() {
        let blocks = render("a `partial code span", &theme());
        assert_eq!(text_of(&blocks[0]), "a `partial code span");
        assert_eq!(blocks[0].spans.len(), 1);
    }

    #[test]
    /// Where the terminal can make the text clickable, only the text is shown: the target
    /// rides in the escape. Where it cannot, the target is printed so it is not lost.
    fn a_link_says_where_it_goes_only_when_it_cannot_be_clicked() {
        let theme = theme();
        let source = "see [the docs](https://example.com) now";

        let clickable = render_linked(source, &theme, 80, &mut Links::new());
        assert_eq!(text_of(&clickable[0]), "see the docs now");

        let blocks = render_linked(source, &theme, 80, &mut Links::disabled());
        assert_eq!(text_of(&blocks[0]), "see the docs (https://example.com) now");

        let text = span_with(&blocks[0], "the docs");
        assert_eq!(text.style.fg, Some(theme.md_link));
        assert!(text.style.add_modifier.contains(Modifier::UNDERLINED));

        let url = span_with(&blocks[0], " (https://example.com)");
        assert_eq!(url.style.fg, Some(theme.md_link_url));
    }

    #[test]
    fn a_link_whose_text_is_its_target_does_not_repeat_it() {
        let blocks = render("[https://example.com](https://example.com)", &theme());
        assert_eq!(text_of(&blocks[0]), "https://example.com");
    }

    #[test]
    fn an_autolinked_address_is_not_repeated_with_its_scheme() {
        let blocks = render("[ramon@example.com](mailto:ramon@example.com)", &theme());
        assert_eq!(text_of(&blocks[0]), "ramon@example.com");
    }

    #[test]
    fn something_that_only_looks_like_a_link_stays_literal() {
        for source in ["[not a link] here", "[text](", "[](https://x)", "[text]()"] {
            let blocks = render(source, &theme());
            assert_eq!(text_of(&blocks[0]), source, "{source}");
        }
    }

    #[test]
    fn a_quote_is_marked_with_a_bar_and_set_in_italic() {
        let theme = theme();
        let blocks = render("> quoted", &theme);
        assert_eq!(text_of(&blocks[0]), "│ quoted");

        assert_eq!(blocks[0].spans[0].style.fg, Some(theme.md_quote_border));
        let body = &blocks[0].spans[1];
        assert_eq!(body.style.fg, Some(theme.md_quote));
        assert!(body.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn a_bare_quote_marker_opens_an_empty_quote_line() {
        assert_eq!(rendered(&render(">", &theme())), vec!["│ "]);
    }

    #[test]
    fn a_rule_spans_the_width_up_to_ohms_cap() {
        let theme = theme();
        let blocks = render_linked("---", &theme, 20, &mut Links::new());
        assert_eq!(text_of(&blocks[0]), "─".repeat(20));
        assert_eq!(blocks[0].spans[0].style.fg, Some(theme.md_hr));

        // Wider terminals stop at ohm's cap of 80.
        assert_eq!(
            text_of(&render_linked("---", &theme, 200, &mut Links::new())[0]).chars().count(),
            80
        );
        assert_eq!(text_of(&render_linked("***", &theme, 5, &mut Links::new())[0]), "─────");
    }

    #[test]
    fn empty_lines_survive_as_empty_blocks() {
        let blocks = render("one\n\ntwo", &theme());
        assert_eq!(blocks.len(), 3);
        assert_eq!(text_of(&blocks[1]), "");
    }

    #[test]
    fn every_markdown_token_is_reachable() {
        let theme = theme();
        let source = "# One\n### Three\n- item\n> quote\n---\n```rs\ncode\n```\n`inline` [t](u)";
        // Rendered without hyperlinks, since that is the path that prints a link's target
        // and so the only one that reaches `md_link_url`.
        let used: Vec<_> = render_linked(source, &theme, 80, &mut Links::disabled())
            .iter()
            .flat_map(|block| block.spans.iter().filter_map(|span| span.style.fg))
            .collect();

        for (name, color) in [
            ("md_heading", theme.md_heading),
            ("md_list_bullet", theme.md_list_bullet),
            ("md_quote", theme.md_quote),
            ("md_quote_border", theme.md_quote_border),
            ("md_hr", theme.md_hr),
            ("md_code_block", theme.md_code_block),
            ("md_code_block_border", theme.md_code_block_border),
            ("md_code", theme.md_code),
            ("md_link", theme.md_link),
            ("md_link_url", theme.md_link_url),
        ] {
            assert!(used.contains(&color), "nothing painted with {name}");
        }
    }
}


