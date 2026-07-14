//! The footer, and the activity line above the editor.
//!
//! ohm's footer is two rows with no fill of its own: where the session is on the first row,
//! what it has spent on the second, with the model pushed to the right edge. The one piece
//! of color in it is the context reading, which warns as the window fills.
//!
//! The counts are cumulative over the session, because that is the number a reader is
//! deciding on; the context reading comes from the last turn alone, because that is what
//! occupies the window right now. The two are carried separately for that reason.

use crate::theme::Theme;
use crate::wrap::text_width;
use crate::wrap::truncate;
use micro_types::Usage;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

/// Spinner frames, advanced once per render tick.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Rows the footer occupies.
pub const HEIGHT: u16 = 2;

/// Columns kept between the counts and the model so the two never run together.
const MIN_GAP: usize = 2;

/// Context readings from here up are called out, the second more loudly than the first.
const CONTEXT_WARNING: f64 = 70.0;
const CONTEXT_ERROR: f64 = 90.0;

/// Shown in place of a model id before one is known.
const NO_MODEL: &str = "no-model";

pub fn spinner_frame(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
}

/// Everything the footer reports.
///
/// Every part beyond the working directory is optional, and one that is absent is simply
/// not drawn rather than leaving a gap where it would have been.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Footer<'a> {
    pub cwd: &'a str,
    /// Git branch of the checkout, when the session is in one.
    pub branch: Option<&'a str>,
    /// The session's own name, when it has been given one.
    pub session: Option<&'a str>,
    /// Tokens summed over every turn of the session.
    pub total: Usage,
    /// Tokens the most recent turn used, which is what fills the context window.
    pub last: Usage,
    pub context_window: u32,
    pub model: &'a str,
    /// Reasoning budget, `"off"` included. Absent when the model does not reason.
    pub thinking: Option<&'a str>,
    /// Images waiting to go with the next prompt, so an attachment is visible before it is
    /// sent rather than only in the notice that announced it.
    pub attachments: usize,
}

impl<'a> Footer<'a> {
    /// The footer's rows, exactly [`HEIGHT`] of them.
    pub fn rows(&self, theme: &Theme, width: usize) -> Vec<Line<'static>> {
        vec![self.place_row(theme, width), self.usage_row(theme, width)]
    }

    /// Where the session is: the working directory, its branch, and its name.
    fn place(&self) -> String {
        let mut out = self.cwd.to_string();
        if let Some(branch) = self.branch.filter(|branch| !branch.is_empty()) {
            out.push_str(&format!(" ({branch})"));
        }
        if let Some(session) = self.session.filter(|session| !session.is_empty()) {
            out.push_str(&format!(" • {session}"));
        }
        // What is going with the next prompt belongs where the session is described, not in
        // the token counts, which are about what has already been spent.
        if self.attachments > 0 {
            out.push_str(&match self.attachments {
                1 => " • 1 image attached".to_string(),
                count => format!(" • {count} images attached"),
            });
        }
        out
    }

    fn place_row(&self, theme: &Theme, width: usize) -> Line<'static> {
        Line::from(vec![Span::styled(
            truncate(&self.place(), width),
            Style::new().fg(theme.dim),
        )])
    }

    /// The counts, then the context reading, then the model against the right edge.
    ///
    /// When the two halves cannot both fit, the model gives way first: the counts are what
    /// changes turn to turn, and the model is on screen elsewhere.
    fn usage_row(&self, theme: &Theme, width: usize) -> Line<'static> {
        let dim = Style::new().fg(theme.dim);

        let mut left = self.count_spans(theme);
        if span_width(&left) > width {
            left = clip_spans(left, width);
        }
        let left_width = span_width(&left);

        let right = self.model_side();
        let room = width.saturating_sub(left_width + MIN_GAP);
        let right = match left_width + MIN_GAP + text_width(&right) <= width {
            true => right,
            false => clip(&right, room),
        };
        let gap = width.saturating_sub(left_width + text_width(&right));

        let mut spans = left;
        if gap > 0 {
            spans.push(Span::styled(" ".repeat(gap), dim));
        }
        if !right.is_empty() {
            spans.push(Span::styled(right, dim));
        }
        Line::from(spans)
    }

    /// The left half: dim counts, then the context reading in whatever color it has earned.
    fn count_spans(&self, theme: &Theme) -> Vec<Span<'static>> {
        let dim = Style::new().fg(theme.dim);
        let mut spans = Vec::new();

        let counts = self.counts();
        if !counts.is_empty() {
            spans.push(Span::styled(counts, dim));
        }
        if let Some(context) = self.context() {
            if !spans.is_empty() {
                spans.push(Span::styled(" ".to_string(), dim));
            }
            spans.push(Span::styled(
                context,
                Style::new().fg(self.context_color(theme)),
            ));
        }
        spans
    }

    fn counts(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (marker, count) in [
            ("↑", self.total.input),
            ("↓", self.total.output),
            ("R", self.total.cache_read),
            ("W", self.total.cache_write),
        ] {
            if count > 0 {
                parts.push(format!("{marker}{}", format_tokens(count)));
            }
        }
        if let Some(rate) = self.cache_hit_rate() {
            parts.push(format!("CH{rate:.1}%"));
        }
        parts.join(" ")
    }

    /// Share of the last turn's prompt that came out of cache. Reported only once the
    /// session has used a cache at all, since a flat 0% before then says nothing.
    fn cache_hit_rate(&self) -> Option<f64> {
        if self.total.cache_read == 0 && self.total.cache_write == 0 {
            return None;
        }
        let prompt = self.last.input + self.last.cache_read + self.last.cache_write;
        (prompt > 0).then(|| (self.last.cache_read as f64 / prompt as f64) * 100.0)
    }

    fn context(&self) -> Option<String> {
        let percent = self.context_percent()?;
        Some(format!(
            "{percent:.1}%/{}",
            format_tokens(self.context_window)
        ))
    }

    fn context_percent(&self) -> Option<f64> {
        context_percent(&self.last, self.context_window)
    }

    fn context_color(&self, theme: &Theme) -> Color {
        match self.context_percent().unwrap_or(0.0) {
            percent if percent > CONTEXT_ERROR => theme.error,
            percent if percent > CONTEXT_WARNING => theme.warning,
            _ => theme.dim,
        }
    }

    /// The model, and the reasoning budget it is running with.
    fn model_side(&self) -> String {
        let model = match self.model.is_empty() {
            true => NO_MODEL,
            false => self.model,
        };
        match self.thinking.filter(|level| !level.is_empty()) {
            Some("off") => format!("{model} • thinking off"),
            Some(level) => format!("{model} • {level}"),
            None => model.to_string(),
        }
    }
}

/// The line above the editor while a turn runs.
pub fn activity_line(
    theme: &Theme,
    tick: usize,
    elapsed: Duration,
    interrupted: bool,
    label: &str,
) -> Line<'static> {
    if interrupted {
        return Line::from(vec![Span::styled(
            "stopping…".to_string(),
            Style::new().fg(theme.warning),
        )]);
    }
    Line::from(vec![
        Span::styled(
            format!("{} ", spinner_frame(tick)),
            Style::new().fg(theme.accent),
        ),
        Span::styled(
            format!("{label}  {}  ", format_elapsed(elapsed)),
            Style::new().fg(theme.muted),
        ),
        Span::styled("ctrl+c to interrupt", Style::new().fg(theme.dim)),
    ])
}

/// Fraction of the context window a turn occupied, if it can be known.
fn context_percent(usage: &Usage, window: u32) -> Option<f64> {
    if window == 0 {
        return None;
    }
    let used = usage.input + usage.cache_read + usage.cache_write + usage.output;
    if used == 0 {
        return None;
    }
    Some((used as f64 / window as f64) * 100.0)
}

/// Token counts, shortened so the footer stays one line.
pub fn format_tokens(count: u32) -> String {
    match count {
        0..=999 => count.to_string(),
        1_000..=9_999 => format!("{:.1}k", count as f64 / 1_000.0),
        10_000..=999_999 => format!("{}k", (count as f64 / 1_000.0).round() as u32),
        _ => format!("{:.1}M", count as f64 / 1_000_000.0),
    }
}

pub fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..=59 => format!("{seconds}s"),
        _ => format!("{}m{:02}s", seconds / 60, seconds % 60),
    }
}

/// Replace the home prefix with `~` so a deep path still fits.
pub fn shorten_home(path: &str) -> String {
    let Some(home) = std::env::var_os("HOME") else {
        return path.to_string();
    };
    let home = home.to_string_lossy();
    match path.strip_prefix(home.as_ref()) {
        Some(rest) => format!("~{rest}"),
        None => path.to_string(),
    }
}

fn span_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|span| text_width(&span.content)).sum()
}

/// `text` cut to `width` columns with nothing to mark the cut.
///
/// The model is cut rather than elided because the head of a model id is the part that
/// identifies it, and two columns spent on an ellipsis are two fewer of the name.
fn clip(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let advance = text_width(grapheme);
        if used + advance > width {
            break;
        }
        out.push_str(grapheme);
        used += advance;
    }
    out
}

fn clip_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0;
    for mut span in spans {
        if used >= width {
            break;
        }
        if used + text_width(&span.content) > width {
            span.content = clip(&span.content, width - used).into();
        }
        used += text_width(&span.content);
        out.push(span);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn footer<'a>() -> Footer<'a> {
        Footer {
            cwd: "~/code/micro",
            model: "claude-opus-5",
            context_window: 200_000,
            ..Footer::default()
        }
    }

    fn usage(input: u32, output: u32, cache_read: u32, cache_write: u32) -> Usage {
        Usage {
            input,
            output,
            cache_read,
            cache_write,
        }
    }

    #[test]
    fn token_counts_shorten_as_they_grow() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_500), "1.5k");
        assert_eq!(format_tokens(42_000), "42k");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }

    #[test]
    fn elapsed_time_switches_to_minutes() {
        assert_eq!(format_elapsed(Duration::from_secs(9)), "9s");
        assert_eq!(format_elapsed(Duration::from_secs(75)), "1m15s");
    }

    #[test]
    fn the_footer_is_two_rows() {
        let rows = footer().rows(&Theme::dark(), 60);
        assert_eq!(rows.len(), HEIGHT as usize);
    }

    #[test]
    fn the_first_row_places_the_session() {
        let mut footer = footer();
        assert_eq!(
            rendered(&footer.rows(&Theme::dark(), 60)[0]),
            "~/code/micro"
        );

        footer.branch = Some("main");
        assert_eq!(
            rendered(&footer.rows(&Theme::dark(), 60)[0]),
            "~/code/micro (main)"
        );

        footer.session = Some("parity work");
        assert_eq!(
            rendered(&footer.rows(&Theme::dark(), 60)[0]),
            "~/code/micro (main) • parity work"
        );
    }

    #[test]
    fn an_empty_branch_or_session_is_not_drawn() {
        let footer = Footer {
            branch: Some(""),
            session: Some(""),
            ..footer()
        };
        assert_eq!(
            rendered(&footer.rows(&Theme::dark(), 60)[0]),
            "~/code/micro"
        );
    }

    #[test]
    fn the_first_row_truncates_rather_than_wrapping() {
        let footer = Footer {
            cwd: "~/a/very/long/working/directory/path/that/keeps/going",
            ..footer()
        };
        let row = &footer.rows(&Theme::dark(), 12)[0];
        assert_eq!(text_width(&rendered(row)), 12);
    }

    #[test]
    fn the_second_row_counts_every_kind_of_token() {
        let footer = Footer {
            total: usage(1_200, 340, 45_000, 2_100),
            last: usage(1_200, 340, 45_000, 2_100),
            ..footer()
        };
        let text = rendered(&footer.rows(&Theme::dark(), 100)[1]);
        assert!(text.starts_with("↑1.2k ↓340 R45k W2.1k CH"), "{text}");
    }

    #[test]
    fn a_count_of_zero_is_left_out() {
        let footer = Footer {
            total: usage(1_200, 340, 0, 0),
            last: usage(1_200, 340, 0, 0),
            ..footer()
        };
        let text = rendered(&footer.rows(&Theme::dark(), 100)[1]);
        assert!(text.starts_with("↑1.2k ↓340 "), "{text}");
        assert!(!text.contains('R'), "{text}");
        assert!(!text.contains('W'), "{text}");
        assert!(!text.contains("CH"), "{text}");
    }

    #[test]
    fn the_cache_hit_rate_comes_from_the_last_turn() {
        let footer = Footer {
            // Cumulative reads are large; the last turn read 800 of a 1,000 token prompt.
            total: usage(20_000, 5_000, 90_000, 1_000),
            last: usage(200, 50, 800, 0),
            ..footer()
        };
        let text = rendered(&footer.rows(&Theme::dark(), 100)[1]);
        assert!(text.contains("CH80.0%"), "{text}");
    }

    #[test]
    fn the_context_reading_is_the_last_turn_against_the_window() {
        let footer = Footer {
            total: usage(100_000, 0, 0, 0),
            last: usage(20_000, 0, 0, 0),
            ..footer()
        };
        let text = rendered(&footer.rows(&Theme::dark(), 100)[1]);
        assert!(text.contains("10.0%/200k"), "{text}");
    }

    #[test]
    fn the_context_reading_warns_as_the_window_fills() {
        let theme = Theme::dark();
        let color = |used: u32| {
            Footer {
                last: usage(used, 0, 0, 0),
                ..footer()
            }
            .context_color(&theme)
        };
        assert_eq!(color(20_000), theme.dim);
        assert_eq!(color(150_000), theme.warning);
        assert_eq!(color(190_000), theme.error);
    }

    #[test]
    fn only_the_context_reading_carries_a_color_of_its_own() {
        let theme = Theme::dark();
        let footer = Footer {
            total: usage(1_000, 100, 0, 0),
            last: usage(190_000, 0, 0, 0),
            ..footer()
        };
        let row = &footer.rows(&theme, 80)[1];
        let colored: Vec<Color> = row
            .spans
            .iter()
            .filter_map(|span| span.style.fg)
            .filter(|color| *color != theme.dim)
            .collect();
        assert_eq!(colored, vec![theme.error]);
    }

    #[test]
    fn the_model_sits_against_the_right_edge() {
        let footer = Footer {
            total: usage(1_200, 340, 0, 0),
            last: usage(1_200, 340, 0, 0),
            ..footer()
        };
        let text = rendered(&footer.rows(&Theme::dark(), 60)[1]);
        assert_eq!(text_width(&text), 60);
        assert!(text.starts_with("↑1.2k"), "{text}");
        assert!(text.ends_with("claude-opus-5"), "{text}");
    }

    #[test]
    fn the_reasoning_budget_follows_the_model() {
        let mut footer = footer();
        assert!(rendered(&footer.rows(&Theme::dark(), 60)[1]).ends_with("claude-opus-5"));

        footer.thinking = Some("high");
        assert!(rendered(&footer.rows(&Theme::dark(), 60)[1]).ends_with("claude-opus-5 • high"));

        footer.thinking = Some("off");
        assert!(
            rendered(&footer.rows(&Theme::dark(), 60)[1]).ends_with("claude-opus-5 • thinking off")
        );
    }

    #[test]
    fn a_session_without_a_model_says_so() {
        let footer = Footer {
            model: "",
            ..footer()
        };
        assert!(rendered(&footer.rows(&Theme::dark(), 40)[1]).ends_with(NO_MODEL));
    }

    #[test]
    fn the_model_gives_way_before_the_counts_do() {
        let footer = Footer {
            total: usage(1_200, 340, 45_000, 2_100),
            last: usage(1_200, 340, 45_000, 2_100),
            ..footer()
        };
        for width in 1..60 {
            let row = &footer.rows(&Theme::dark(), width)[1];
            let text = rendered(row);
            assert!(
                text_width(&text) <= width,
                "row of {} exceeds {width}: {text}",
                text_width(&text)
            );
        }

        // Wide enough for the counts and nothing else: the model is gone, the counts are not.
        let text = rendered(&footer.rows(&Theme::dark(), 26)[1]);
        assert!(text.starts_with("↑1.2k"), "{text}");
        assert!(!text.contains("claude"), "{text}");
    }

    #[test]
    fn neither_row_ever_exceeds_the_width() {
        let footer = Footer {
            cwd: "~/a/deep/path/somewhere",
            branch: Some("feature/a-long-branch-name"),
            session: Some("a named session"),
            total: usage(1_234_567, 89_000, 45_000, 2_100),
            last: usage(190_000, 1_000, 0, 0),
            thinking: Some("xhigh"),
            ..footer()
        };
        for width in 1..120 {
            for row in footer.rows(&Theme::dark(), width) {
                let drawn = text_width(&rendered(&row));
                assert!(drawn <= width, "row of {drawn} exceeds {width}");
            }
        }
    }

    #[test]
    fn a_fresh_session_reports_nothing_it_does_not_know() {
        let footer = Footer {
            context_window: 0,
            ..footer()
        };
        let text = rendered(&footer.rows(&Theme::dark(), 60)[1]);
        assert_eq!(text.trim(), "claude-opus-5");
    }

    #[test]
    fn clipping_keeps_the_head_and_never_overruns() {
        assert_eq!(clip("claude-opus-5", 6), "claude");
        assert_eq!(clip("claude", 0), "");
        assert_eq!(clip("日本語", 3), "日");
    }

    #[test]
    fn the_spinner_cycles() {
        assert_eq!(spinner_frame(0), spinner_frame(SPINNER.len()));
        assert_ne!(spinner_frame(0), spinner_frame(1));
    }

    #[test]
    fn the_activity_line_shows_the_elapsed_time() {
        let line = activity_line(&Theme::dark(), 0, Duration::from_secs(12), false, "working");
        assert!(rendered(&line).contains("12s"));
        assert!(rendered(&line).contains("working"));
        let line = activity_line(&Theme::dark(), 0, Duration::from_secs(12), true, "working");
        assert_eq!(rendered(&line), "stopping…");
    }
}
