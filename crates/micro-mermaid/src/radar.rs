//! Radar charts, drawn as one row per axis rather than as a polygon.
//!
//! A radar chart is really a polygon plotted on spokes around a centre, and
//! a terminal has no circle to put one on — the closest a grid of cells
//! gets is an octagon built from a dozen special cases, which would be a
//! worse lie than admitting the shape is gone. What a reader actually wants
//! from a radar chart is which curve is ahead on which axis, and that
//! survives the trip to a terminal perfectly well as a row per axis: the
//! axis named on the left, a shared value scale drawn across it, and each
//! curve's value marked on that scale with its own glyph, the way
//! `journey.rs` marks a task's score against its own fixed scale. Comparing
//! two curves is then reading along one row rather than around a shape.

use crate::canvas::{draw_text, Canvas};
use crate::labels::{ascii_lower, clean_label, strip_controls};
use crate::types::Cls;
use crate::width::string_width;

/// Axes past this and the chart is refused: past a certain count the rows
/// are a table, and a table is better read as the source it came from.
const MAX_AXES: usize = 12;

/// Curves past this and the chart is refused: this is also how many marker
/// glyphs are defined, and reusing one for a seventh curve would make two
/// different curves unreadable from each other.
const MAX_CURVES: usize = MARKERS.len();

/// One glyph per curve, assigned in declaration order.
const MARKERS: [&str; 6] = ["●", "○", "■", "□", "▲", "△"];

/// Columns the shared value scale is drawn across.
const SCALE_WIDTH: usize = 24;

struct Axis {
    label: String,
}

struct Curve {
    label: String,
    values: Vec<f64>,
}

struct Chart {
    title: Option<String>,
    axes: Vec<Axis>,
    curves: Vec<Curve>,
    min: f64,
    max: f64,
}

/// Draw `src` as a radar chart, or answer nothing when it is not one.
pub(crate) fn render_radar(src: &str) -> Option<Canvas> {
    let chart = parse_radar(src)?;
    Some(draw(&chart))
}

fn parse_radar(src: &str) -> Option<Chart> {
    let src = strip_controls(src);
    let statements = crate::parse::statements_of(&src);
    let header = statements.first()?;
    if ascii_lower(header.split_whitespace().next().unwrap_or("")) != "radar-beta" {
        return None;
    }

    let mut title = None;
    let mut axes: Vec<Axis> = Vec::new();
    let mut curves: Vec<Curve> = Vec::new();
    let mut min: Option<f64> = None;
    let mut max: Option<f64> = None;

    for st in &statements[1..] {
        let word = st.split_whitespace().next()?;
        let rest = st[word.len()..].trim();
        match ascii_lower(word).as_str() {
            "title" => title = Some(clean_label(rest)),
            "axis" => {
                for token in rest.split(',') {
                    if axes.len() >= MAX_AXES {
                        return None;
                    }
                    let (id, label) = read_id_and_label(token.trim())?;
                    axes.push(Axis {
                        label: label.unwrap_or(id),
                    });
                }
            }
            "curve" => {
                if curves.len() >= MAX_CURVES {
                    return None;
                }
                curves.push(read_curve(rest)?);
            }
            "max" => max = Some(rest.parse().ok()?),
            "min" => min = Some(rest.parse().ok()?),
            _ => return None,
        }
    }

    if axes.is_empty() || curves.is_empty() {
        return None;
    }
    // Every curve has to name a value for every axis — a radar chart with a
    // curve missing an axis has nothing sensible to plot there.
    if curves.iter().any(|c| c.values.len() != axes.len()) {
        return None;
    }

    let min = min.unwrap_or(0.0);
    let max = max.unwrap_or_else(|| {
        curves
            .iter()
            .flat_map(|c| c.values.iter().copied())
            .fold(f64::MIN, f64::max)
    });
    if !min.is_finite() || !max.is_finite() || max <= min {
        return None;
    }

    Some(Chart {
        title,
        axes,
        curves,
        min,
        max,
    })
}

/// `name["Alice"]{1,2,3}`: an id, an optional label, and one value per axis.
fn read_curve(rest: &str) -> Option<Curve> {
    let open = rest.find('{')?;
    let (id, label) = read_id_and_label(rest[..open].trim())?;
    let body = rest[open + 1..].trim_end().strip_suffix('}')?;
    let values: Option<Vec<f64>> = body.split(',').map(|v| v.trim().parse().ok()).collect();
    let values = values?;
    if values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some(Curve {
        label: label.unwrap_or(id),
        values,
    })
}

/// `id` or `id["Label"]`.
fn read_id_and_label(s: &str) -> Option<(String, Option<String>)> {
    match s.find('[') {
        Some(open) => {
            let id = s[..open].trim();
            if id.is_empty() {
                return None;
            }
            let rest = &s[open + 1..];
            let close = rest.rfind(']')?;
            Some((id.to_string(), Some(clean_label(&rest[..close]))))
        }
        None => {
            let id = s.trim();
            if id.is_empty() {
                None
            } else {
                Some((id.to_string(), None))
            }
        }
    }
}

fn draw(chart: &Chart) -> Canvas {
    let top = usize::from(chart.title.is_some());
    let scale_row = top;
    let axes_start = scale_row + 1;
    let legend_row = axes_start + chart.axes.len() + 1;
    let rows = legend_row + 1;

    let label_w = chart.axes.iter().map(|a| string_width(&a.label)).max().unwrap_or(0);
    let track_x = label_w + 1;
    let scale_text = format!("scale {}–{}", trim_number(chart.min), trim_number(chart.max));
    let legend_w = legend_width(&chart.curves);

    let width = (track_x + SCALE_WIDTH)
        .max(string_width(chart.title.as_deref().unwrap_or("")))
        .max(string_width(&scale_text))
        .max(legend_w)
        .max(1);

    let mut canvas = Canvas::new(width, rows);
    if let Some(title) = &chart.title {
        draw_text(&mut canvas, title, 0, 0, Cls::Title);
    }
    draw_text(&mut canvas, &scale_text, 0, scale_row, Cls::EdgeLabel);

    let track: String = "─".repeat(SCALE_WIDTH);
    for (i, axis) in chart.axes.iter().enumerate() {
        let y = axes_start + i;
        draw_text(&mut canvas, &axis.label, 0, y, Cls::Text);
        draw_text(&mut canvas, &track, track_x, y, Cls::Edge);
        for (ci, curve) in chart.curves.iter().enumerate() {
            let pos = scale_pos(curve.values[i], chart.min, chart.max);
            canvas.set(track_x + pos, y, MARKERS[ci], Cls::EdgeLabel);
        }
    }

    draw_legend(&mut canvas, &chart.curves, legend_row);
    canvas
}

/// Column a value lands on within `SCALE_WIDTH`, clamped to the track so an
/// out-of-range value still shows at whichever end it overshot.
fn scale_pos(value: f64, min: f64, max: f64) -> usize {
    let frac = ((value.clamp(min, max) - min) / (max - min)).clamp(0.0, 1.0);
    (frac * (SCALE_WIDTH - 1) as f64).round() as usize
}

fn legend_width(curves: &[Curve]) -> usize {
    let mut w = 0;
    for (i, curve) in curves.iter().enumerate() {
        if i > 0 {
            w += 2;
        }
        w += string_width(MARKERS[i]) + 1 + string_width(&curve.label);
    }
    w
}

fn draw_legend(canvas: &mut Canvas, curves: &[Curve], y: usize) {
    let mut x = 0;
    for (i, curve) in curves.iter().enumerate() {
        if i > 0 {
            x += 2;
        }
        draw_text(canvas, MARKERS[i], x, y, Cls::EdgeLabel);
        x += string_width(MARKERS[i]) + 1;
        draw_text(canvas, &curve.label, x, y, Cls::Text);
        x += string_width(&curve.label);
    }
}

/// A value written the way it was meant: whole numbers without a decimal
/// point.
fn trim_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_radar(src).expect("it is a radar chart").to_lines().plain
    }

    /// Each axis gets its own row, and each curve's value is marked on that
    /// row's shared scale with its own glyph.
    #[test]
    fn each_axis_is_a_row_with_every_curves_value_marked() {
        let rows = drawn(
            "radar-beta\n\
             title Skills\n\
             axis a[\"Communication\"], b[\"Technical\"]\n\
             curve name1[\"Alice\"]{0,5}\n\
             curve name2[\"Bob\"]{5,0}\n\
             max 5\n\
             min 0",
        );
        assert_eq!(rows[0], "Skills");
        let comm_row = rows.iter().find(|r| r.contains("Communication")).unwrap();
        // Alice is 0 on this axis (left end) and Bob is 5 (right end), so
        // Alice's glyph comes first on the row.
        assert!(comm_row.find('●').unwrap() < comm_row.find('○').unwrap(), "{comm_row:?}");
        let tech_row = rows.iter().find(|r| r.contains("Technical")).unwrap();
        assert!(tech_row.find('○').unwrap() < tech_row.find('●').unwrap(), "{tech_row:?}");
    }

    /// The legend names which glyph belongs to which curve.
    #[test]
    fn the_legend_names_each_curves_glyph() {
        let rows = drawn(
            "radar-beta\n\
             axis a[\"A\"]\n\
             curve x[\"Alice\"]{1}\n\
             curve y[\"Bob\"]{2}",
        );
        let legend = rows.last().unwrap();
        assert!(legend.contains('●') && legend.contains("Alice"), "{legend:?}");
        assert!(legend.contains('○') && legend.contains("Bob"), "{legend:?}");
    }

    /// With no `max` given, the scale stretches to the highest value any
    /// curve actually reaches.
    #[test]
    fn the_scale_defaults_to_the_highest_value_present() {
        let rows = drawn("radar-beta\n  axis a[\"A\"]\n  curve x[\"X\"]{40}");
        assert!(rows.iter().any(|r| r.contains("scale 0–40")), "{rows:?}");
    }

    /// Anything that is not a radar chart, or is one but malformed, is
    /// refused rather than guessed at.
    #[test]
    fn what_is_not_a_radar_chart_is_left_alone() {
        assert!(render_radar("graph TD\n A --> B").is_none());
        assert!(render_radar("radar-beta").is_none(), "no axes or curves");
        assert!(render_radar("radar-beta\n  axis a[\"A\"]").is_none(), "no curves");
        assert!(
            render_radar("radar-beta\n  axis a[\"A\"], b[\"B\"]\n  curve x[\"X\"]{1}").is_none(),
            "a curve missing a value for one of the axes"
        );
    }

    /// A chart with more curves than there are marker glyphs is refused
    /// rather than reusing a glyph for two different curves.
    #[test]
    fn too_many_curves_are_refused() {
        let mut source = String::from("radar-beta\n  axis a[\"A\"]\n");
        for index in 0..MAX_CURVES + 1 {
            source.push_str(&format!("  curve c{index}[\"C{index}\"]{{1}}\n"));
        }
        assert!(render_radar(&source).is_none());
    }
}
