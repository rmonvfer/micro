//! XY charts, drawn as bars and stepped lines against a pair of axes.
//!
//! The plot itself is unglamorous — a left gutter of value ticks, a bottom
//! row of category ticks, bars as filled columns, lines as points joined by
//! the same right-angle jog flowchart edges already use between ranks. The
//! one real decision is what a value turns into vertically: everything is
//! scaled once, from the axis range to a fixed row budget, so a bar's height
//! and a line's rise mean the same thing on the same plot.

use crate::canvas::{draw_text, Canvas};
use crate::labels::{ascii_lower, clean_label, fit_label, strip_controls};
use crate::types::Cls;
use crate::width::string_width;

/// Points a single series may carry before the chart is refused.
const MAX_POINTS: usize = 128;
/// Series (bar or line statements together) past this and there is nothing
/// left to read as a chart.
const MAX_SERIES: usize = 16;
/// Rows the plot's value axis is scaled into.
const PLOT_H: usize = 15;
const MAX_CANVAS_CELLS: usize = 1 << 21;

enum SeriesKind {
    Bar,
    Line,
}

struct Series {
    kind: SeriesKind,
    values: Vec<f64>,
}

struct Chart {
    title: Option<String>,
    x_title: Option<String>,
    x_categories: Vec<String>,
    y_title: Option<String>,
    y_range: Option<(f64, f64)>,
    series: Vec<Series>,
}

/// Draw `src` as an xy chart, or answer nothing when it is not one.
pub(crate) fn render_xychart(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let mut lines = src.lines().map(str::trim).filter(|l| !l.is_empty());

    let header = lines.next()?;
    if ascii_lower(header.split_whitespace().next()?) != "xychart-beta" {
        return None;
    }
    // `horizontal` asks for the chart to be laid on its side; understood, but
    // the terminal reads a chart the same width either way, so orientation is
    // always the one below, the same way `pie`'s `showData` is read but does
    // not change what gets drawn.

    let mut chart = Chart {
        title: None,
        x_title: None,
        x_categories: Vec::new(),
        y_title: None,
        y_range: None,
        series: Vec::new(),
    };
    for line in lines {
        apply(line, &mut chart)?;
    }
    if chart.series.is_empty() {
        return None;
    }
    draw(&chart)
}

fn apply(line: &str, chart: &mut Chart) -> Option<()> {
    let word = line.split_whitespace().next()?;
    let first = ascii_lower(word);
    let rest = line[word.len()..].trim();

    match first.as_str() {
        "title" => {
            chart.title = Some(clean_label(rest));
            Some(())
        }
        "x-axis" => {
            let (title, body) = take_quoted_prefix(rest);
            chart.x_title = title;
            if let Some(list) = body.strip_prefix('[').and_then(|b| b.strip_suffix(']')) {
                chart.x_categories = list.split(',').map(|c| clean_label(c.trim())).collect();
                Some(())
            } else if body.is_empty() {
                Some(())
            } else {
                parse_range(body).map(|_| ())
            }
        }
        "y-axis" => {
            let (title, body) = take_quoted_prefix(rest);
            chart.y_title = title;
            if body.is_empty() {
                return Some(());
            }
            chart.y_range = Some(parse_range(body)?);
            Some(())
        }
        "bar" => {
            let values = parse_values(rest)?;
            push_series(chart, SeriesKind::Bar, values)
        }
        "line" => {
            let values = parse_values(rest)?;
            push_series(chart, SeriesKind::Line, values)
        }
        _ => None,
    }
}

fn push_series(chart: &mut Chart, kind: SeriesKind, values: Vec<f64>) -> Option<()> {
    if chart.series.len() >= MAX_SERIES || values.is_empty() || values.len() > MAX_POINTS {
        return None;
    }
    chart.series.push(Series { kind, values });
    Some(())
}

/// A leading `"quoted title"`, if there is one, and whatever follows it.
fn take_quoted_prefix(s: &str) -> (Option<String>, &str) {
    let Some(rest) = s.strip_prefix('"') else {
        return (None, s);
    };
    match rest.find('"') {
        Some(end) => (Some(clean_label(&rest[..end])), rest[end + 1..].trim()),
        None => (None, s),
    }
}

/// `min --> max`.
fn parse_range(s: &str) -> Option<(f64, f64)> {
    let (lo, hi) = s.split_once("-->")?;
    let lo: f64 = lo.trim().parse().ok()?;
    let hi: f64 = hi.trim().parse().ok()?;
    if !lo.is_finite() || !hi.is_finite() || lo >= hi {
        return None;
    }
    Some((lo, hi))
}

/// `[1, 2.5, -3]`.
fn parse_values(s: &str) -> Option<Vec<f64>> {
    let inner = s.strip_prefix('[')?.strip_suffix(']')?;
    inner
        .split(',')
        .map(|v| v.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect()
}

fn data_range(chart: &Chart) -> (f64, f64) {
    if let Some(range) = chart.y_range {
        return range;
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for series in &chart.series {
        for &v in &series.values {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    // A flat series still needs a span to divide by.
    if (hi - lo).abs() < f64::EPSILON {
        hi = lo + 1.0;
    }
    (lo, hi)
}

fn trim_number(value: f64) -> String {
    match value.fract() == 0.0 && value.abs() < 1e15 {
        true => format!("{}", value as i64),
        false => format!("{value:.1}"),
    }
}

fn draw(chart: &Chart) -> Option<Canvas> {
    let (lo, hi) = data_range(chart);
    let span = (hi - lo).max(f64::EPSILON);
    // 0 maps to whichever row is closest to it inside the visible range, so a
    // bar always grows from the zero baseline when zero is on the plot at all.
    let row_for = |v: f64| -> usize {
        let t = ((v - lo) / span).clamp(0.0, 1.0);
        (((PLOT_H - 1) as f64) * (1.0 - t)).round() as usize
    };
    let baseline = row_for(lo.max(0.0).min(hi));

    let points = chart
        .series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0);
    let bar_count = chart
        .series
        .iter()
        .filter(|s| matches!(s.kind, SeriesKind::Bar))
        .count();
    let slot_w = bar_count.max(1) + 3;

    let min_label = trim_number(lo);
    let max_label = trim_number(hi);
    let gutter = string_width(&min_label).max(string_width(&max_label));
    let plot_left = gutter + 1;
    let plot_w = points * slot_w;

    let header_rows = usize::from(chart.title.is_some()) + usize::from(chart.y_title.is_some());
    let footer_rows =
        1 + usize::from(!chart.x_categories.is_empty()) + usize::from(chart.x_title.is_some());
    let canvas_w = (plot_left + plot_w + 1)
        .max(plot_left + string_width(chart.y_title.as_deref().unwrap_or("")));
    let canvas_h = header_rows + PLOT_H + footer_rows;
    if canvas_w.saturating_mul(canvas_h) > MAX_CANVAS_CELLS || points == 0 {
        return None;
    }

    let mut canvas = Canvas::new(canvas_w, canvas_h);
    let mut row0 = 0;
    if let Some(title) = &chart.title {
        draw_text(&mut canvas, title, 0, row0, Cls::Title);
        row0 += 1;
    }
    if let Some(y_title) = &chart.y_title {
        draw_text(&mut canvas, y_title, 0, row0, Cls::Text);
        row0 += 1;
    }
    let plot_top = row0;
    let plot_bottom = plot_top + PLOT_H - 1;

    draw_text(
        &mut canvas,
        &max_label,
        plot_left.saturating_sub(string_width(&max_label) + 1),
        plot_top,
        Cls::EdgeLabel,
    );
    draw_text(
        &mut canvas,
        &min_label,
        plot_left.saturating_sub(string_width(&min_label) + 1),
        plot_bottom,
        Cls::EdgeLabel,
    );
    canvas.seg_v(plot_left, plot_top, plot_bottom);
    canvas.seg_h(plot_bottom, plot_left, plot_left + plot_w);

    let slot_x = |i: usize| plot_left + 1 + i * slot_w;

    let mut bar_series = 0usize;
    for series in &chart.series {
        match series.kind {
            SeriesKind::Bar => {
                for (i, &v) in series.values.iter().enumerate() {
                    let vr = row_for(v);
                    // A value that lands exactly on the baseline has no
                    // height to show, the same way a pie slice worth nothing
                    // keeps its row but draws no bar into it.
                    if vr == baseline {
                        continue;
                    }
                    let x = slot_x(i) + bar_series;
                    let top = plot_top + vr.min(baseline);
                    let bottom = plot_top + vr.max(baseline);
                    for y in top..=bottom {
                        canvas.set(x, y, "█", Cls::Border);
                    }
                }
                bar_series += 1;
            }
            SeriesKind::Line => {
                let xs: Vec<usize> = (0..series.values.len())
                    .map(|i| slot_x(i) + bar_count / 2)
                    .collect();
                let ys: Vec<usize> = series
                    .values
                    .iter()
                    .map(|&v| plot_top + row_for(v))
                    .collect();
                for w in 1..xs.len() {
                    connect(&mut canvas, xs[w - 1], ys[w - 1], xs[w], ys[w]);
                }
                for (&x, &y) in xs.iter().zip(&ys) {
                    canvas.set(x, y, "●", Cls::Edge);
                }
            }
        }
    }

    let mut foot = plot_bottom + 1;
    if !chart.x_categories.is_empty() {
        for (i, cat) in chart.x_categories.iter().enumerate().take(points) {
            let text = fit_label(cat, slot_w.saturating_sub(1));
            let x = slot_x(i) + (slot_w.saturating_sub(1)).saturating_sub(string_width(&text)) / 2;
            draw_text(&mut canvas, &text, x, foot, Cls::Text);
        }
        foot += 1;
    }
    if let Some(x_title) = &chart.x_title {
        let x = plot_left + plot_w.saturating_sub(string_width(x_title)) / 2;
        draw_text(&mut canvas, x_title, x, foot, Cls::Text);
    }

    canvas.finalize_mask();
    Some(canvas)
}

/// Join two plotted points with a right-angle jog through their shared
/// midpoint column, the same shape flowchart edges route between ranks.
fn connect(canvas: &mut Canvas, x0: usize, y0: usize, x1: usize, y1: usize) {
    if x1 <= x0 {
        return;
    }
    if y0 == y1 {
        canvas.seg_h(y0, x0, x1);
        return;
    }
    let mid = x0 + (x1 - x0) / 2;
    canvas.seg_h(y0, x0, mid);
    canvas.seg_v(mid, y0, y1);
    canvas.seg_h(y1, mid, x1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_xychart(src)
            .expect("it is an xy chart")
            .to_lines()
            .plain
    }

    /// A bar chart draws one filled column per value, each reaching a height
    /// proportional to where it sits between the axis's min and max.
    #[test]
    fn bars_grow_from_the_axis_minimum() {
        let rows = drawn("xychart-beta\n  y-axis 0 --> 10\n  bar [10, 5, 0]");
        let heights: Vec<usize> = (0..3)
            .map(|col_group| {
                rows.iter()
                    .filter(|r| r.chars().nth(4 + col_group * 4).is_some_and(|c| c == '█'))
                    .count()
            })
            .collect();
        assert!(
            heights[0] > heights[1] && heights[1] > heights[2],
            "{heights:?}: {rows:?}"
        );
        assert_eq!(heights[2], 0, "a zero value draws no bar: {rows:?}");
    }

    /// Category labels drawn under `x-axis [a, b, c]` line up with their bars.
    #[test]
    fn category_labels_sit_under_the_axis() {
        let rows = drawn("xychart-beta\n  x-axis [Jan, Feb, Mar]\n  bar [1, 2, 3]");
        let joined = rows.join("\n");
        assert!(joined.contains("Jan"), "{rows:?}");
        assert!(joined.contains("Feb"), "{rows:?}");
        assert!(joined.contains("Mar"), "{rows:?}");
    }

    /// A line series is drawn as markers joined by right-angle jogs, and the
    /// y-axis ticks show the range it was told to use.
    #[test]
    fn a_line_series_is_plotted_as_joined_points() {
        let rows = drawn("xychart-beta\n  y-axis \"Score\" 0 --> 100\n  line [0, 50, 100]");
        let joined = rows.join("\n");
        assert_eq!(joined.matches('●').count(), 3);
        assert!(joined.contains("Score"), "{rows:?}");
        assert!(joined.contains('0'), "{rows:?}");
        assert!(joined.contains("100"), "{rows:?}");
    }

    /// With no explicit y-axis, the range is taken from the data itself.
    #[test]
    fn the_y_axis_auto_scales_from_the_data_when_not_given() {
        let rows = drawn("xychart-beta\n  bar [4, 8, 2]");
        assert!(rows.iter().any(|r| r.contains('8')), "{rows:?}");
    }

    /// `title` labels the chart, drawn above the plot.
    #[test]
    fn a_title_is_drawn_above_the_plot() {
        let rows = drawn("xychart-beta\n  title Revenue\n  bar [1, 2]");
        assert_eq!(rows[0], "Revenue");
    }

    /// Two bar series in the same chart are drawn as adjacent columns within
    /// each category's slot, so neither one is drawn over the other.
    #[test]
    fn two_bar_series_draw_side_by_side() {
        let rows = drawn("xychart-beta\n  y-axis 0 --> 10\n  bar [10, 0]\n  bar [0, 10]");
        let bottom = rows.iter().find(|r| r.contains('█')).unwrap();
        let cols: Vec<usize> = bottom
            .chars()
            .enumerate()
            .filter(|&(_, c)| c == '█')
            .map(|(i, _)| i)
            .collect();
        assert_eq!(cols.len(), 2, "two distinct bar columns: {rows:?}");
        assert!(cols[1] > cols[0] + 1, "{cols:?}: {rows:?}");
    }

    /// Anything that is not an xy chart, or breaks down partway through, is
    /// refused rather than drawn wrong.
    #[test]
    fn what_is_not_an_xychart_is_left_alone() {
        assert!(render_xychart("graph TD\n A --> B").is_none());
        assert!(render_xychart("xychart-beta").is_none(), "no series at all");
        assert!(render_xychart("xychart-beta\n  bar [1, nonsense]").is_none());
        assert!(
            render_xychart("xychart-beta\n  y-axis 10 --> 0").is_none(),
            "an inverted range"
        );
        assert!(
            render_xychart("xychart-beta\n  bar []").is_none(),
            "an empty series"
        );
    }

    /// A series with more points than can be read as a shape is refused.
    #[test]
    fn too_many_points_are_refused() {
        let values: Vec<String> = (0..MAX_POINTS + 1).map(|i| i.to_string()).collect();
        let src = format!("xychart-beta\n  bar [{}]", values.join(", "));
        assert!(render_xychart(&src).is_none());
    }
}
