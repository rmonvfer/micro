//! Width-aware text wrapping.

use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Columns a grapheme cluster occupies.
pub fn grapheme_width(grapheme: &str) -> usize {
    if grapheme == "\t" {
        return 1;
    }
    grapheme.width()
}

/// Columns `text` occupies once rendered.
pub fn text_width(text: &str) -> usize {
    text.graphemes(true).map(grapheme_width).sum()
}

/// `text` shortened to `width` columns, marking the cut with an ellipsis when one fits.
pub fn truncate(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_string();
    }
    match width {
        0 => String::new(),
        1 => "…".to_string(),
        _ => {
            let mut out = String::new();
            let mut used = 0;
            for grapheme in text.graphemes(true) {
                let advance = grapheme_width(grapheme);
                if used + advance > width - 1 {
                    break;
                }
                out.push_str(grapheme);
                used += advance;
            }
            out.push('…');
            out
        }
    }
}

/// Byte ranges partitioning `text` into rows no wider than `width` columns.
pub fn wrap_ranges(text: &str, width: usize) -> Vec<Range<usize>> {
    let width = width.max(1);
    let mut rows: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;
    let mut column = 0usize;

    let mut break_at: Option<usize> = None;
    let mut previous_was_space = false;

    for (index, grapheme) in text.grapheme_indices(true) {
        let is_space = is_whitespace(grapheme);
        if previous_was_space && !is_space && index > start {
            break_at = Some(index);
        }
        previous_was_space = is_space;

        let advance = grapheme_width(grapheme);
        if column + advance > width && index > start {
            let cut = break_at.filter(|at| *at > start).unwrap_or(index);
            rows.push(start..cut);
            start = cut;
            break_at = None;
            column = text_width(&text[start..index]);
        }
        column += advance;
    }

    rows.push(start..text.len());
    rows
}

/// Wrap styled spans into display lines no wider than `width` columns.
pub fn wrap_spans(spans: &[Span<'static>], width: usize, indent: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let indent = indent.min(width.saturating_sub(1));

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut column = 0usize;

    for token in tokenize(spans) {
        let token_width = text_width(&token.text);

        if token.is_space {
            if current.is_empty() && !lines.is_empty() {
                continue;
            }
            if current.is_empty() && lines.is_empty() && column + token_width > width {
                continue;
            }
            if column + token_width > width {
                flush(&mut lines, &mut current, &mut column, indent);
                continue;
            }
            column += token_width;
            current.push(Span::styled(token.text, token.style));
            continue;
        }

        if column + token_width > width && !current.is_empty() {
            flush(&mut lines, &mut current, &mut column, indent);
        }

        if column + token_width <= width {
            column += token_width;
            current.push(Span::styled(token.text, token.style));
            continue;
        }

        for chunk in split_to_width(
            &token.text,
            width.saturating_sub(column),
            width.saturating_sub(indent),
        ) {
            if column + text_width(&chunk) > width && !current.is_empty() {
                flush(&mut lines, &mut current, &mut column, indent);
            }
            column += text_width(&chunk);
            current.push(Span::styled(chunk, token.style));
        }
    }

    if !current.is_empty() || lines.is_empty() {
        trim_trailing_space(&mut current);
        lines.push(Line::from(current));
    }
    lines
}

/// Wrap styled spans at the width, without regard for word boundaries.
pub fn wrap_spans_hard(spans: &[Span<'static>], width: usize, indent: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let indent = indent.min(width.saturating_sub(1));

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut column = 0usize;

    for span in spans {
        let mut chunk = String::new();
        for grapheme in span.content.graphemes(true) {
            let advance = grapheme_width(grapheme);
            if column + advance > width {
                if !chunk.is_empty() {
                    current.push(Span::styled(std::mem::take(&mut chunk), span.style));
                }
                lines.push(Line::from(std::mem::take(&mut current)));
                column = indent;
                if indent > 0 {
                    current.push(Span::raw(" ".repeat(indent)));
                }
            }
            chunk.push_str(grapheme);
            column += advance;
        }
        if !chunk.is_empty() {
            current.push(Span::styled(chunk, span.style));
        }
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

/// Close the row being built and open the next one at the hanging indent.
fn flush(
    lines: &mut Vec<Line<'static>>,
    current: &mut Vec<Span<'static>>,
    column: &mut usize,
    indent: usize,
) {
    trim_trailing_space(current);
    lines.push(Line::from(std::mem::take(current)));
    if indent > 0 {
        current.push(Span::raw(" ".repeat(indent)));
    }
    *column = indent;
}

/// Whitespace left at a wrap point would paint the row's background past its last character.
fn trim_trailing_space(spans: &mut Vec<Span<'static>>) {
    while let Some(last) = spans.last_mut() {
        let trimmed = last.content.trim_end();
        if trimmed.is_empty() {
            spans.pop();
            continue;
        }
        if trimmed.len() != last.content.len() {
            last.content = trimmed.to_string().into();
        }
        break;
    }
}

/// Break `text` into pieces: the first fits `first` columns, the rest fit `rest` columns.
fn split_to_width(text: &str, first: usize, rest: usize) -> Vec<String> {
    let rest = rest.max(1);
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    let mut budget = first.max(1);
    let mut column = 0usize;

    for grapheme in text.graphemes(true) {
        let advance = grapheme_width(grapheme);
        if column + advance > budget && !chunk.is_empty() {
            chunks.push(std::mem::take(&mut chunk));
            budget = rest;
            column = 0;
        }
        chunk.push_str(grapheme);
        column += advance;
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

fn is_whitespace(grapheme: &str) -> bool {
    !grapheme.is_empty() && grapheme.chars().all(char::is_whitespace)
}

struct Token {
    text: String,
    style: Style,
    is_space: bool,
}

/// Split spans into alternating word and whitespace tokens, keeping each token's style.
fn tokenize(spans: &[Span<'static>]) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    for span in spans {
        let style = span.style;
        let mut current = String::new();
        let mut current_is_space = false;

        for grapheme in span.content.graphemes(true) {
            let is_space = is_whitespace(grapheme);
            if !current.is_empty() && is_space != current_is_space {
                tokens.push(Token {
                    text: std::mem::take(&mut current),
                    style,
                    is_space: current_is_space,
                });
            }
            current_is_space = is_space;
            current.push_str(grapheme);
        }
        if !current.is_empty() {
            tokens.push(Token {
                text: current,
                style,
                is_space: current_is_space,
            });
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn short_text_stays_on_one_line() {
        let lines = wrap_spans(&[Span::raw("hello there")], 40, 0);
        assert_eq!(rendered(&lines), vec!["hello there"]);
    }

    #[test]
    fn wrapping_breaks_on_word_boundaries() {
        let lines = wrap_spans(&[Span::raw("the quick brown fox jumps")], 10, 0);
        assert_eq!(rendered(&lines), vec!["the quick", "brown fox", "jumps"]);
    }

    #[test]
    fn continuation_lines_carry_the_indent() {
        let lines = wrap_spans(&[Span::raw("alpha beta gamma")], 8, 2);
        assert_eq!(rendered(&lines), vec!["alpha", "  beta", "  gamma"]);
    }

    #[test]
    fn no_wrapped_row_exceeds_the_width() {
        let text = "supercalifragilistic expialidocious and a few short words";
        for width in 4..24 {
            for line in wrap_spans(&[Span::raw(text)], width, 3) {
                let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert!(
                    text_width(&rendered) <= width,
                    "row {rendered:?} exceeds width {width}"
                );
            }
        }
    }

    #[test]
    fn leading_indentation_on_the_first_row_is_kept() {
        let lines = wrap_spans(&[Span::raw("  indented text")], 20, 4);
        assert_eq!(rendered(&lines), vec!["  indented text"]);
    }

    #[test]
    fn hard_wrapping_breaks_at_the_edge_and_keeps_spacing() {
        let lines = wrap_spans_hard(&[Span::raw("    indented code line")], 10, 4);
        assert_eq!(
            rendered(&lines),
            vec!["    indent", "    ed cod", "    e line"]
        );
    }

    #[test]
    fn no_hard_wrapped_row_exceeds_the_width() {
        let text = "let very_long_identifier = another_very_long_identifier(argument);";
        for width in 3..40 {
            for line in wrap_spans_hard(&[Span::raw(text)], width, 2) {
                let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert!(text_width(&rendered) <= width);
            }
        }
    }

    #[test]
    fn a_word_wider_than_the_row_is_split() {
        let lines = wrap_spans(&[Span::raw("abcdefghijkl")], 5, 0);
        assert_eq!(rendered(&lines), vec!["abcde", "fghij", "kl"]);
    }

    #[test]
    fn styles_survive_wrapping() {
        let styled = Style::new().fg(ratatui::style::Color::Red);
        let lines = wrap_spans(
            &[Span::raw("plain "), Span::styled("colored words", styled)],
            8,
            0,
        );
        let colored: String = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .filter(|span| span.style == styled)
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(colored, "coloredwords");
    }

    #[test]
    fn empty_input_still_produces_one_line() {
        assert_eq!(wrap_spans(&[], 10, 0).len(), 1);
    }

    #[test]
    fn wrap_ranges_partition_the_whole_string() {
        let text = "one two three four five";
        let ranges = wrap_ranges(text, 9);
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, text.len());
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }

    #[test]
    fn wrap_ranges_prefer_word_boundaries() {
        let text = "one two three";
        let rows: Vec<&str> = wrap_ranges(text, 7)
            .into_iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(rows, vec!["one ", "two ", "three"]);
    }

    #[test]
    fn wrap_ranges_split_an_oversized_word() {
        let text = "abcdefgh";
        let rows: Vec<&str> = wrap_ranges(text, 3)
            .into_iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(rows, vec!["abc", "def", "gh"]);
    }

    #[test]
    fn wrap_ranges_of_empty_text_is_one_empty_row() {
        assert_eq!(wrap_ranges("", 10), vec![0..0]);
    }

    #[test]
    fn wide_graphemes_count_two_columns() {
        assert_eq!(text_width("日本"), 4);
        assert_eq!(wrap_ranges("日本語", 4).len(), 2);
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(truncate("hello", 1), "…");
        assert_eq!(truncate("hello", 0), "");
    }
}
