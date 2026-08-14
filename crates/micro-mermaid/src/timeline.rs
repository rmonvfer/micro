//! Timelines, drawn as periods down a spine with what happened beside them.
//!
//! A timeline is a sequence of periods, each holding however many events. Drawn with the
//! periods down the left against a vertical spine and their events to the right of it, the
//! order is the order they are read in, and a period holding four events is plainly four
//! events rather than four periods — which is the mistake a flat list invites.

use crate::canvas::draw_text;
use crate::canvas::Canvas;
use crate::labels::{clean_label, strip_controls};
use crate::types::Cls;
use crate::width::string_width;

/// Periods past this and the timeline is refused: past a certain length it is a list, and
/// a list is better read as the source it was written as.
const MAX_PERIODS: usize = 64;

/// Columns between the period and the spine, and between the spine and the events.
const GAP: usize = 1;

struct Period {
    label: String,
    events: Vec<String>,
}

/// Draw `src` as a timeline, or answer nothing when it is not one.
pub(crate) fn render_timeline(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let mut lines = src.lines().map(str::trim).filter(|line| !line.is_empty());

    if lines.next()? != "timeline" {
        return None;
    }

    let mut title = None;
    let mut periods: Vec<Period> = Vec::new();
    for line in lines {
        if let Some(named) = line.strip_prefix("title ") {
            title = Some(clean_label(named.trim()));
            continue;
        }
        // A section names a stretch of the timeline; it reads as a period holding whatever
        // follows it, which is how it is drawn.
        let line = line.strip_prefix("section ").unwrap_or(line);

        // `period : event : event`, and a line with no colon is a period on its own.
        let mut parts = line
            .split(':')
            .map(str::trim)
            .filter(|part| !part.is_empty());
        let label = clean_label(parts.next()?);
        if label.is_empty() {
            return None;
        }
        let events: Vec<String> = parts.map(clean_label).filter(|e| !e.is_empty()).collect();

        if periods.len() >= MAX_PERIODS {
            return None;
        }
        periods.push(Period { label, events });
    }

    if periods.is_empty() {
        return None;
    }
    Some(draw(title.as_deref(), &periods))
}

fn draw(title: Option<&str>, periods: &[Period]) -> Canvas {
    let widest_period = periods
        .iter()
        .map(|period| string_width(&period.label))
        .max()
        .unwrap_or(0);
    let widest_event = periods
        .iter()
        .flat_map(|period| period.events.iter())
        .map(|event| string_width(event))
        .max()
        .unwrap_or(0);

    // A period with no events still takes a row, because it happened.
    let rows: usize = periods
        .iter()
        .map(|period| period.events.len().max(1))
        .sum();
    let top = usize::from(title.is_some());

    let spine = widest_period + GAP;
    let events_at = spine + 1 + GAP;
    let width = (events_at + widest_event).max(string_width(title.unwrap_or("")));
    let mut canvas = Canvas::new(width.max(1), rows + top);

    if let Some(title) = title {
        draw_text(&mut canvas, title, 0, 0, Cls::Title);
    }

    let mut y = top;
    for (index, period) in periods.iter().enumerate() {
        let height = period.events.len().max(1);
        draw_text(&mut canvas, &period.label, 0, y, Cls::Text);

        for row in 0..height {
            // The spine runs the whole way down, so the periods read as one sequence
            // rather than as separate lists that happen to be stacked. A row with
            // something coming off it branches; the last row of the last period
            // closes the spine; anything else is the spine passing through.
            let last = index + 1 == periods.len() && row + 1 == height;
            let branches = row == 0 || period.events.get(row).is_some();
            let glyph = match (last, branches) {
                (true, _) => "└",
                (false, true) => "├",
                (false, false) => "│",
            };
            draw_text(&mut canvas, glyph, spine, y + row, Cls::Edge);

            if let Some(event) = period.events.get(row) {
                draw_text(&mut canvas, "─", spine + 1, y + row, Cls::Edge);
                draw_text(&mut canvas, event, events_at, y + row, Cls::EdgeLabel);
            }
        }
        y += height;
    }

    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_timeline(src)
            .expect("it is a timeline")
            .to_lines()
            .plain
    }

    /// Periods run down the left against a spine, with their events beside them.
    #[test]
    fn a_period_holds_its_events_beside_it() {
        let rows = drawn("timeline\n  title History\n  2020 : Started : Grew\n  2021 : Shipped");
        assert_eq!(rows[0], "History");
        assert!(rows[1].starts_with("2020"), "{rows:?}");
        assert!(rows[1].contains("Started"), "{rows:?}");
        // The second event of 2020 belongs to it, so it takes a row with no period beside it.
        assert!(rows[2].contains("Grew"), "{rows:?}");
        assert!(!rows[2].contains("2021"), "{rows:?}");
        assert!(
            rows[3].contains("2021") && rows[3].contains("Shipped"),
            "{rows:?}"
        );
    }

    /// The spine runs the whole way down and closes at the end, so the periods read as one
    /// sequence rather than as stacked lists.
    #[test]
    fn the_spine_runs_through_and_closes() {
        let rows = drawn("timeline\n  One : a\n  Two : b");
        assert!(rows[0].contains('├'), "{rows:?}");
        assert!(rows[1].contains('└'), "the last row closes it: {rows:?}");
    }

    /// A period nothing happened in still happened.
    #[test]
    fn a_period_with_no_events_keeps_its_row() {
        let rows = drawn("timeline\n  Quiet year\n  Loud year : Something");
        assert!(rows[0].starts_with("Quiet year"), "{rows:?}");
        assert_eq!(rows.len(), 2, "{rows:?}");
    }

    /// What is not a timeline is refused rather than guessed at.
    #[test]
    fn what_is_not_a_timeline_is_left_alone() {
        assert!(render_timeline("graph TD\n  A --> B").is_none());
        assert!(render_timeline("timeline").is_none(), "nothing happened");
        assert!(render_timeline("timeline\n  title Only a title").is_none());
    }

    /// A timeline longer than this is a list, and reads better as the source it came from.
    #[test]
    fn too_many_periods_are_refused() {
        let mut source = String::from("timeline\n");
        for index in 0..MAX_PERIODS + 1 {
            source.push_str(&format!("  Year {index} : Something\n"));
        }
        assert!(render_timeline(&source).is_none());
    }
}
