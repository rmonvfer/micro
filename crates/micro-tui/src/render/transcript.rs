//! The scrollback, laid out as display lines.
//!
//! Everything is wrapped to the region's width here rather than by a widget, so the number
//! of lines produced is exactly the number the frame has to find room for. Alongside the
//! lines it reports where each entry begins, which is what lets a whole entry be handed to
//! the terminal's own scrollback without cutting one in half.
//!
//! ohm separates one kind of message from another by the color of the ground it sits on
//! rather than by a glyph in front of it: a prompt and a tool result each get a band of
//! background, and an answer gets none, which is what makes the answer the thing on the
//! page. [`band`] is that shape, and both callers go through it.

use crate::markdown;
use crate::render::tool;
use crate::theme::Theme;
use crate::transcript::Entry;
use crate::transcript::NoticeLevel;
use crate::transcript::Transcript;
use crate::wrap::wrap_spans;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

/// How the transcript should be shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Display {
    pub width: usize,
    pub show_thinking: bool,
    /// Entry index the reader has selected, if any.
    pub focus: Option<usize>,
    /// First entry to draw. Everything before it has already gone to the terminal's own
    /// scrollback and must not be drawn a second time.
    pub from: usize,
    /// Whether this terminal can make text clickable.
    pub hyperlinks: bool,
    /// How this terminal draws an image, when it can.
    pub images: Option<crate::capabilities::ImageProtocol>,
}

/// The transcript as drawn, with the first line of each entry.
#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    /// Every link the transcript drew, for the frame to make clickable once the lines have
    /// been placed and their columns are settled.
    pub links: crate::render::links::Links,
    /// Every image it reserved room for, drawn once the rows are placed.
    pub pictures: crate::render::pictures::Pictures,
}

/// Render the transcript into display lines, from [`Display::from`] onward.
pub fn lines(transcript: &Transcript, theme: &Theme, display: &Display) -> Rendered {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut pictures = crate::render::pictures::Pictures::new(display.images);
    let mut links = match display.hyperlinks {
        true => crate::render::links::Links::new(),
        false => crate::render::links::Links::disabled(),
    };
    let entries = transcript.entries();
    let from = display.from.min(entries.len());

    for (index, entry) in entries.iter().enumerate().skip(from) {
        let before = out.len();
        // An entry keeps the blank row above it whether the entry before it is on screen or
        // in the scrollback, so a block reads the same either way.
        if !out.is_empty() || index > 0 {
            out.push(Line::default());
        }
        let start = out.len();

        match entry {
            Entry::User(text) => push_user(&mut out, text, theme, display),
            Entry::Assistant(assistant) => {
                push_thinking(&mut out, &assistant.thinking, theme, display);
                // An answer arriving is marked by the spinner in the status rows, not by a
                // block on the text. ohm draws no cursor into the transcript.
                push_markdown(&mut out, &assistant.text, theme, display, &mut links);
                if let Some(error) = &assistant.error {
                    push_notice(&mut out, error, NoticeLevel::Error, theme, display);
                }
            }
            Entry::Tool(entry) => out.extend(tool::lines(
                entry,
                display.focus == Some(index),
                theme,
                display.width,
            )),
            Entry::Compaction { summary, expanded } => {
                push_compaction(&mut out, summary, *expanded, theme, display)
            }
            Entry::Image { data, mime_type } => {
                push_image(&mut out, data, mime_type, theme, display, &mut pictures)
            }
            Entry::Notice { text, level } => push_notice(&mut out, text, *level, theme, display),
        }

        // A banded entry carries its own inset inside the tint; everything else is pushed in
        // by the same column so text lines up whether or not it sits on coloured ground.
        for line in out.iter_mut().skip(start) {
            let banded = line.spans.first().is_some_and(|span| span.style.bg.is_some());
            if !banded {
                *line = indented(std::mem::take(line));
            }
        }

        // An entry that drew nothing takes no room, and leaves no gap behind it either.
        if out.len() == start {
            out.truncate(before);
        }
    }

    Rendered {
        lines: out,
        links,
        pictures,
    }
}

/// Wrap `rows` in the box ohm draws around a message: a blank row above and below, and
/// every row tinted across the full width so the block reads as one card.
///
/// The transcript is already drawn a column in from each edge, and that column is the inset
/// ohm's box applies to its own contents, so the rows go in without further padding and
/// land in the column ohm puts them in.
pub(super) fn band(
    rows: Vec<Line<'static>>,
    width: usize,
    background: Color,
) -> Vec<Line<'static>> {
    std::iter::once(Line::default())
        .chain(rows)
        .chain(std::iter::once(Line::default()))
        .map(|line| super::tint(indented(line), width + PADDING * 2, background))
        .collect()
}

/// Columns of ground either side of a message's text, inside its own band.
pub(super) const PADDING: usize = 1;

/// Push a row in from the left edge, so text sits inside the band rather than against it.
fn indented(line: Line<'static>) -> Line<'static> {
    match line.spans.is_empty() {
        true => line,
        false => {
            let mut spans = vec![Span::raw(" ".repeat(PADDING))];
            spans.extend(line.spans);
            Line::from(spans)
        }
    }
}

/// A prompt sits in a band of its own color, which is how ohm marks it: no glyph in front
/// of it and no author beside it, just the ground it is written on.
fn push_user(out: &mut Vec<Line<'static>>, text: &str, theme: &Theme, display: &Display) {
    if text.trim().is_empty() {
        return;
    }
    let body = Style::new().fg(theme.user_message_text);
    let mut rows: Vec<Line<'static>> = Vec::new();
    for line in text.split('\n') {
        let spans = vec![Span::styled(line.to_string(), body)];
        rows.extend(wrap_spans(&spans, display.width, 0));
    }
    out.extend(band(rows, display.width, theme.user_message_bg));
}

/// Reasoning is background information: folded behind a label unless asked for.
fn push_thinking(out: &mut Vec<Line<'static>>, thinking: &str, theme: &Theme, display: &Display) {
    if thinking.trim().is_empty() {
        return;
    }
    // Reasoning is italic body text in `thinkingText`, carrying no glyph of its own — it is
    // marked out by being italic and dim, the way ohm marks it.
    let style = theme.thinking();

    if display.show_thinking {
        for line in thinking.split('\n') {
            let spans = vec![Span::styled(line.to_string(), style)];
            out.extend(wrap_spans(&spans, display.width, 0));
        }
        return;
    }

    // Hidden, a whole run collapses to one fixed label rather than the latest line. A live
    // tail reads as content the model produced; a label reads as what it is, a fold.
    out.push(Line::from(vec![Span::styled("Thinking...", style)]));
}

/// An image, given the rows it needs and drawn into them by the terminal.
///
/// The rows are held empty here; the escape that fills them goes on after layout, since it
/// occupies no columns and cannot be measured as text. A terminal that cannot draw images
/// gets a description instead, which is at least honest about what was attached.
fn push_image(
    out: &mut Vec<Line<'static>>,
    data: &str,
    mime_type: &str,
    theme: &Theme,
    display: &Display,
    pictures: &mut crate::render::pictures::Pictures,
) {
    let Some(reserved) = pictures.reserve(data, display.width) else {
        let bytes = data.len() / 4 * 3;
        out.push(Line::from(vec![Span::styled(
            format!("[{mime_type}, {}]", crate::app::human_size(bytes)),
            Style::new().fg(theme.muted),
        )]));
        return;
    };
    for _ in 0..reserved {
        out.push(Line::default());
    }
}

/// A stretch of conversation replaced by a summary, drawn as a labelled card.
///
/// Folded it says only what it stands for and how to open it; opened it shows the summary
/// the model wrote. Either way it sits in its own tint, because it is neither a prompt nor
/// an answer — it is the conversation reporting on itself.
fn push_compaction(
    out: &mut Vec<Line<'static>>,
    summary: &str,
    expanded: bool,
    theme: &Theme,
    display: &Display,
) {
    let mut rows = vec![Line::from(vec![Span::styled(
        "[compaction]",
        Style::new()
            .fg(theme.custom_message_label)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )])];
    rows.push(Line::default());

    let body = Style::new().fg(theme.custom_message_text);
    match expanded {
        true => {
            for line in summary.split('\n') {
                let spans = vec![Span::styled(line.to_string(), body)];
                rows.extend(wrap_spans(&spans, display.width, 0));
            }
        }
        false => rows.push(Line::from(vec![
            Span::styled(
                format!("Compacted from {} ", approximate_tokens(summary)),
                body,
            ),
            Span::styled("(ctrl+o", Style::new().fg(theme.dim)),
            Span::styled(" to expand)", body),
        ])),
    }

    out.extend(band(rows, display.width, theme.custom_message_bg));
}

/// Roughly how much the summarized stretch was worth, from what stands in for it.
///
/// The real figure belongs to the compactor and never reaches the interface, so this says
/// what it can honestly say: the size of the summary itself.
fn approximate_tokens(summary: &str) -> String {
    let tokens = summary.chars().count() / 4;
    match tokens {
        0..=999 => format!("{tokens} tokens"),
        _ => format!("{}.{}k tokens", tokens / 1000, (tokens % 1000) / 100),
    }
}

fn push_markdown(
    out: &mut Vec<Line<'static>>,
    text: &str,
    theme: &Theme,
    display: &Display,
    links: &mut crate::render::links::Links,
) {
    // A response almost always ends with a newline, and the empty row that would produce
    // reads as a gap the model asked for rather than punctuation.
    let text = text.trim_end_matches('\n');
    if text.is_empty() {
        return;
    }
    // An answer is written straight onto the terminal's own ground. A fenced block is
    // marked by its fences and by the color of its text, not by a fill behind it.
    for block in markdown::render_linked(text, theme, display.width, links) {
        out.extend(wrap_spans(&block.spans, display.width, block.indent));
    }
}

fn push_notice(
    out: &mut Vec<Line<'static>>,
    text: &str,
    level: NoticeLevel,
    theme: &Theme,
    display: &Display,
) {
    // No glyph. A notice is coloured text at the same inset as everything else, and a
    // severity worth naming names itself in words rather than in a symbol.
    let (color, prefix) = match level {
        NoticeLevel::Info => (theme.dim, ""),
        NoticeLevel::Warning => (theme.warning, "Warning: "),
        NoticeLevel::Error => (theme.error, "Error: "),
    };
    let body = match text.starts_with(prefix) {
        true => text.to_string(),
        false => format!("{prefix}{text}"),
    };
    let spans = vec![Span::styled(body, Style::new().fg(color))];
    out.extend(wrap_spans(&spans, display.width, 0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrap::text_width;
    use micro_types::AgentEvent;
    use micro_types::StreamEvent;
    use serde_json::json;

    fn display(width: usize) -> Display {
        Display {
            width,
            show_thinking: false,
            focus: None,
            from: 0,
            hyperlinks: true,
            images: None,
        }
    }

    fn rendered(transcript: &Transcript, display: &Display) -> Vec<String> {
        lines(transcript, &Theme::dark(), display)
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    // The band's own padding column is not content; dropping exactly it
                    // keeps every assertion about relative indentation honest.
                    .strip_prefix(' ')
                    .unwrap_or_default()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn a_user_prompt_sits_in_a_band_with_no_prefix() {
        let mut transcript = Transcript::new();
        transcript.push_user("one two three four");
        assert_eq!(
            rendered(&transcript, &display(10)),
            vec!["", "one two", "three four", ""]
        );
    }

    #[test]
    fn the_band_behind_a_prompt_covers_every_cell() {
        let theme = Theme::dark();
        let mut transcript = Transcript::new();
        transcript.push_user("hello");

        for line in lines(&transcript, &theme, &display(30)).lines {
            let width: usize = line.spans.iter().map(|s| text_width(&s.content)).sum();
            assert_eq!(width, 30 + PADDING * 2, "the band reaches both edges");
            assert!(
                line.spans
                    .iter()
                    .all(|span| span.style.bg == Some(theme.user_message_bg)),
                "every cell of a prompt carries the band"
            );
        }
    }

    #[test]
    fn a_prompt_keeps_its_own_line_breaks() {
        let mut transcript = Transcript::new();
        transcript.push_user("first\n\nthird");
        assert_eq!(
            rendered(&transcript, &display(40)),
            vec!["", "first", "", "third", ""]
        );
    }

    #[test]
    fn an_empty_prompt_draws_nothing() {
        let mut transcript = Transcript::new();
        transcript.push_user("   ");
        assert!(lines(&transcript, &Theme::dark(), &display(40))
            .lines
            .is_empty());
    }

    #[test]
    fn an_answer_is_written_on_the_terminals_own_ground() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::MessageDelta {
            event: StreamEvent::TextDelta {
                index: 0,
                delta: "plain words".into(),
            },
        });
        transcript.close();

        for line in lines(&transcript, &Theme::dark(), &display(40)).lines {
            assert!(
                line.spans.iter().all(|span| span.style.bg.is_none()),
                "an answer carries no band"
            );
        }
    }

    #[test]
    fn entries_are_separated_by_a_blank_line() {
        let mut transcript = Transcript::new();
        transcript.push_user("first");
        transcript.push_user("second");
        assert_eq!(
            rendered(&transcript, &display(40)),
            vec!["", "first", "", "", "", "second", ""]
        );
    }

    #[test]
    fn consecutive_tool_calls_are_separated_like_everything_else() {
        let mut transcript = Transcript::new();
        for id in ["a", "b"] {
            transcript.apply(&AgentEvent::ToolStart {
                id: id.into(),
                name: "ls".into(),
                arguments: json!({ "path": "." }),
            });
        }
        // Each result is a three-row band, with one blank row between the two.
        assert_eq!(rendered(&transcript, &display(40)).len(), 7);
    }

    #[test]
    fn each_entry_reports_where_it_starts() {
        let mut transcript = Transcript::new();
        transcript.push_user("first");
        transcript.push_user("second");
        transcript.apply(&AgentEvent::ToolStart {
            id: "a".into(),
            name: "ls".into(),
            arguments: json!({ "path": "." }),
        });

        let out = lines(&transcript, &Theme::dark(), &display(40));
        // Three rows to a band, one blank row between them.
        assert_eq!(out.lines.len(), 11);
    }

    /// Hidden reasoning collapses to a fixed label, not to whatever the model most recently
    /// said. A live tail reads as content; a label reads as a fold.
    #[test]
    fn a_compaction_is_labelled_and_folded_until_asked_for() {
        let mut transcript = Transcript::new();
        transcript.push_compaction("what came before, in short");

        let folded = rendered(&transcript, &display(60));
        assert!(folded.iter().any(|line| line.contains("[compaction]")));
        assert!(folded.iter().any(|line| line.contains("ctrl+o to expand")));
        assert!(
            !folded.iter().any(|line| line.contains("what came before")),
            "the summary itself stays folded"
        );

        transcript.set_all_expanded(true);
        let opened = rendered(&transcript, &display(60));
        assert!(opened.iter().any(|line| line.contains("what came before")));
    }

    /// A summary is recognised by its wrapper, so it is never drawn as a prompt the user
    /// typed — which is what it would look like otherwise, since it arrives as a user
    /// message.
    #[test]
    fn a_summary_message_is_not_shown_as_a_prompt() {
        let transcript = Transcript::from_messages(&[micro_context::summary_message("gist")]);
        assert!(matches!(
            transcript.entries().first(),
            Some(crate::transcript::Entry::Compaction { .. })
        ));
    }

    #[test]
    fn hidden_thinking_collapses_to_a_fixed_label() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::MessageDelta {
            event: StreamEvent::ThinkingDelta {
                index: 0,
                delta: "first thought\nsecond thought".into(),
            },
        });
        assert_eq!(rendered(&transcript, &display(60))[0], "Thinking...");
    }

    #[test]
    fn expanded_thinking_shows_every_line_without_a_glyph() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::MessageDelta {
            event: StreamEvent::ThinkingDelta {
                index: 0,
                delta: "one\ntwo".into(),
            },
        });
        let mut display = display(60);
        display.show_thinking = true;
        let out = rendered(&transcript, &display);
        assert_eq!(out[0], "one");
        assert_eq!(out[1], "two");
    }

    #[test]
    fn a_fenced_block_keeps_its_fences() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::MessageDelta {
            event: StreamEvent::TextDelta {
                index: 0,
                delta: "```rust\nlet x = 1;\n```".into(),
            },
        });
        transcript.close();

        assert_eq!(
            rendered(&transcript, &display(30)),
            vec!["```rust", "  let x = 1;", "```"]
        );
    }

    #[test]
    fn no_row_ever_exceeds_the_width() {
        let mut transcript = Transcript::new();
        transcript.push_user("a prompt long enough to need wrapping at any sensible width");
        transcript.apply(&AgentEvent::MessageDelta {
            event: StreamEvent::TextDelta {
                index: 0,
                delta: format!(
                    "```\n{}\n```\n\nand some prose after it",
                    "abcdefghij".repeat(6)
                ),
            },
        });
        transcript.close();

        for width in 4..60 {
            for line in lines(&transcript, &Theme::dark(), &display(width)).lines {
                let drawn: usize = line.spans.iter().map(|s| text_width(&s.content)).sum();
                // A row is the content plus the column of ground either side of it.
                let frame = width + PADDING * 2;
                assert!(drawn <= frame, "row of {drawn} exceeds {frame}");
            }
        }
    }

    #[test]
    fn an_empty_transcript_renders_nothing() {
        let out = lines(&Transcript::new(), &Theme::dark(), &display(40));
        assert!(out.lines.is_empty());
    }
}

