//! Quadrant charts: a bordered grid split into four by a labelled cross, with named points plotted
//! on it.

use crate::canvas::{draw_text, Canvas, D, L, R, U};
use crate::labels::{ascii_lower, clean_label, fit_label, strip_controls, wrap_label};
use crate::types::Cls;
use crate::width::string_width;

/// Points past this and a scatter of them says nothing a terminal can show clearly.
const MAX_POINTS: usize = 64;

const GRID_W: usize = 43;
const GRID_H: usize = 17;

/// Lines a quadrant name wraps to before it starts crowding the points.
const QUADRANT_LABEL_LINES: usize = 3;

/// One end of an axis range, e.g.
#[derive(Default)]
struct AxisLabel {
    low: Option<String>,
    high: Option<String>,
}

struct Point {
    label: String,
    x: f64,
    y: f64,
}

struct Chart {
    title: Option<String>,
    x_axis: AxisLabel,
    y_axis: AxisLabel,

    quadrants: [Option<String>; 4],
    points: Vec<Point>,
}

/// Draw `src` as a quadrant chart, or answer nothing when it is not one.
pub(crate) fn render_quadrant(src: &str) -> Option<Canvas> {
    let chart = parse_quadrant(src)?;
    Some(draw_quadrant(&chart))
}

fn parse_quadrant(src: &str) -> Option<Chart> {
    let src = strip_controls(src);
    let statements = crate::parse::statements_of(&src);
    let header = statements.first()?;
    if ascii_lower(header.split_whitespace().next().unwrap_or("")) != "quadrantchart" {
        return None;
    }

    let mut title = None;
    let mut x_axis = AxisLabel::default();
    let mut y_axis = AxisLabel::default();
    let mut quadrants: [Option<String>; 4] = Default::default();
    let mut points: Vec<Point> = Vec::new();

    for st in &statements[1..] {
        let word = st.split_whitespace().next()?;
        let rest = st[word.len()..].trim();
        match ascii_lower(word).as_str() {
            "title" => title = Some(clean_label(rest)),
            "x-axis" => x_axis = parse_axis(rest),
            "y-axis" => y_axis = parse_axis(rest),
            "quadrant-1" => quadrants[0] = non_empty(clean_label(rest)),
            "quadrant-2" => quadrants[1] = non_empty(clean_label(rest)),
            "quadrant-3" => quadrants[2] = non_empty(clean_label(rest)),
            "quadrant-4" => quadrants[3] = non_empty(clean_label(rest)),
            _ => {
                if points.len() >= MAX_POINTS {
                    return None;
                }
                points.push(parse_point(st)?);
            }
        }
    }

    if points.is_empty() {
        return None;
    }
    Some(Chart {
        title,
        x_axis,
        y_axis,
        quadrants,
        points,
    })
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// `Low Reach --> High Reach`, or a bare `Reach` with no arrow.
fn parse_axis(rest: &str) -> AxisLabel {
    if let Some((low, high)) = rest.split_once("-->") {
        AxisLabel {
            low: non_empty(clean_label(low.trim())),
            high: non_empty(clean_label(high.trim())),
        }
    } else {
        AxisLabel {
            low: None,
            high: non_empty(clean_label(rest.trim())),
        }
    }
}

/// `Name: [x, y]`.
fn parse_point(st: &str) -> Option<Point> {
    let (name, rest) = st.split_once(':')?;
    let name = clean_label(name.trim());
    let inner = rest.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (x_str, y_str) = inner.split_once(',')?;
    let x: f64 = x_str.trim().parse().ok()?;
    let y: f64 = y_str.trim().parse().ok()?;
    if name.is_empty() || !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(Point { label: name, x, y })
}

fn draw_quadrant(chart: &Chart) -> Canvas {
    let top = usize::from(chart.title.is_some());

    let grid_top = top + 1;
    let grid_bottom = grid_top + GRID_H - 1;
    let y_low_row = grid_bottom + 1;
    let x_label_row = y_low_row + 1;
    let rows = x_label_row + 1;
    let width = GRID_W.max(string_width(chart.title.as_deref().unwrap_or("")));

    let mut canvas = Canvas::new(width, rows);
    if let Some(t) = &chart.title {
        draw_text(&mut canvas, t, 0, 0, Cls::Title);
    }
    if let Some(high) = &chart.y_axis.high {
        draw_centered(&mut canvas, high, 0, GRID_W, top, Cls::EdgeLabel);
    }
    if let Some(low) = &chart.y_axis.low {
        draw_centered(&mut canvas, low, 0, GRID_W, y_low_row, Cls::EdgeLabel);
    }
    let mid = GRID_W / 2;
    if let Some(low) = &chart.x_axis.low {
        let text = fit_label(low, mid.saturating_sub(2));
        draw_text(&mut canvas, &text, 1, x_label_row, Cls::EdgeLabel);
    }
    if let Some(high) = &chart.x_axis.high {
        let text = fit_label(high, (GRID_W - mid).saturating_sub(2));
        let x = (GRID_W - 1).saturating_sub(string_width(&text));
        draw_text(&mut canvas, &text, x, x_label_row, Cls::EdgeLabel);
    }

    canvas.blit(&draw_grid(chart), 0, grid_top);
    canvas
}

fn draw_centered(canvas: &mut Canvas, text: &str, x0: usize, w: usize, y: usize, cls: Cls) {
    let text = fit_label(text, w.saturating_sub(2));
    let x = x0 + w.saturating_sub(string_width(&text)) / 2;
    draw_text(canvas, &text, x, y, cls);
}

fn draw_grid(chart: &Chart) -> Canvas {
    let mut g = Canvas::new(GRID_W, GRID_H);
    let right = GRID_W - 1;
    let bottom = GRID_H - 1;
    let mid_x = GRID_W / 2;
    let mid_y = GRID_H / 2;

    g.set(0, 0, "┌", Cls::Border);
    g.set(right, 0, "┐", Cls::Border);
    g.set(0, bottom, "└", Cls::Border);
    g.set(right, bottom, "┘", Cls::Border);
    for x in 1..right {
        g.add_bits(x, 0, L | R, Cls::Border);
        g.add_bits(x, bottom, L | R, Cls::Border);
    }
    for y in 1..bottom {
        g.add_bits(0, y, U | D, Cls::Border);
        g.add_bits(right, y, U | D, Cls::Border);
    }

    g.seg_v(mid_x, 0, bottom);
    g.seg_h(mid_y, 0, right);
    g.finalize_mask();

    draw_quadrant_names(&mut g, &chart.quadrants, mid_x, mid_y, right, bottom);
    draw_points(&mut g, &chart.points, mid_x, mid_y, right, bottom);
    g
}

/// `quadrant-1` through `quadrant-4` go top-right, top-left, bottom-left, bottom-right.
fn draw_quadrant_names(
    g: &mut Canvas,
    quadrants: &[Option<String>; 4],
    mid_x: usize,
    mid_y: usize,
    right: usize,
    bottom: usize,
) {
    let regions = [
        (
            mid_x + 1,
            right.saturating_sub(1),
            1,
            mid_y.saturating_sub(1),
        ),
        (1, mid_x.saturating_sub(1), 1, mid_y.saturating_sub(1)),
        (
            1,
            mid_x.saturating_sub(1),
            mid_y + 1,
            bottom.saturating_sub(1),
        ),
        (
            mid_x + 1,
            right.saturating_sub(1),
            mid_y + 1,
            bottom.saturating_sub(1),
        ),
    ];
    for (name, (x0, x1, y0, y1)) in quadrants.iter().zip(regions) {
        let Some(name) = name else { continue };
        if x1 < x0 || y1 < y0 {
            continue;
        }
        let w = x1 - x0 + 1;
        let h = y1 - y0 + 1;
        let lines = wrap_label(name, w, h.min(QUADRANT_LABEL_LINES));
        let top = y0 + h.saturating_sub(lines.len()) / 2;
        for (i, line) in lines.iter().enumerate() {
            let x = x0 + w.saturating_sub(string_width(line)) / 2;
            draw_text(g, line, x, top + i, Cls::Text);
        }
    }
}

/// Plot every point at its normalised `(x, y)` position.
fn draw_points(
    g: &mut Canvas,
    points: &[Point],
    mid_x: usize,
    mid_y: usize,
    right: usize,
    bottom: usize,
) {
    for p in points {
        let xn = p.x.clamp(0.0, 1.0);
        let yn = p.y.clamp(0.0, 1.0);
        let mut col = 1 + (xn * (right.saturating_sub(2)) as f64).round() as usize;

        let mut row = 1 + ((1.0 - yn) * (bottom.saturating_sub(2)) as f64).round() as usize;

        if col == mid_x {
            col = if xn >= 0.5 {
                mid_x + 1
            } else {
                mid_x.saturating_sub(1)
            };
        }
        if row == mid_y {
            row = if yn >= 0.5 {
                mid_y.saturating_sub(1)
            } else {
                mid_y + 1
            };
        }
        let (top_bound, bottom_bound) = if row <= mid_y {
            (1, mid_y.saturating_sub(1))
        } else {
            (mid_y + 1, bottom.saturating_sub(1))
        };

        let row = nearby_clear_row(g, col.saturating_sub(1), 3, row, top_bound, bottom_bound);
        draw_text(g, "●", col, row, Cls::Text);

        let (half_lo, half_hi) = if col <= mid_x {
            (1, mid_x.saturating_sub(1))
        } else {
            (mid_x + 1, right.saturating_sub(1))
        };
        let space_right = half_hi.saturating_sub(col + 1);
        let label = fit_label(&p.label, space_right.max(1));
        let label_w = string_width(&label);

        let lx = if col + 2 + label_w <= half_hi {
            col + 2
        } else {
            col.saturating_sub(1 + label_w).max(half_lo)
        };

        let ly = nearby_clear_row(g, lx, label_w, row, top_bound, bottom_bound);
        draw_text(g, &label, lx, ly, Cls::Text);
    }
}

fn nearby_clear_row(
    g: &Canvas,
    x: usize,
    w: usize,
    preferred: usize,
    lo: usize,
    hi: usize,
) -> usize {
    if row_is_clear(g, x, w, preferred) {
        return preferred;
    }
    for step in 1..=3usize {
        if preferred >= lo + step {
            let y = preferred - step;
            if row_is_clear(g, x, w, y) {
                return y;
            }
        }
        let y = preferred + step;
        if y <= hi && row_is_clear(g, x, w, y) {
            return y;
        }
    }
    preferred
}

/// Whether `w` cells starting at `(x, y)` are all still unpainted.
fn row_is_clear(g: &Canvas, x: usize, w: usize, y: usize) -> bool {
    if y >= g.h {
        return false;
    }
    (x..x + w).all(|cx| cx < g.w && g.ch[g.idx(cx, y)] == " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_quadrant(src)
            .expect("it is a quadrant chart")
            .to_lines()
            .plain
    }

    /// The grid is one continuous bordered box split into four by a cross, not four separate boxes.
    #[test]
    fn the_grid_is_a_bordered_box_split_into_four_by_a_cross() {
        let rows = drawn(
            "quadrantChart\n\
             title Campaigns\n\
             x-axis Low Reach --> High Reach\n\
             y-axis Low Engagement --> High Engagement\n\
             quadrant-1 Expand\n\
             quadrant-2 Promote\n\
             quadrant-3 Re-evaluate\n\
             quadrant-4 Improve\n\
             Campaign A: [0.3, 0.6]",
        );
        assert_eq!(rows[0], "Campaigns");
        let top_border = rows.iter().find(|r| r.contains('┌')).expect("top border");
        assert!(top_border.starts_with('┌'), "{top_border:?}");
        assert!(top_border.ends_with('┐'), "{top_border:?}");
        assert!(
            top_border.contains('┬'),
            "the cross tees into the top border: {top_border:?}"
        );
        let bottom_border = rows
            .iter()
            .rev()
            .find(|r| r.contains('└'))
            .expect("bottom border");
        assert!(bottom_border.contains('┴'), "{bottom_border:?}");

        let interior_row = rows
            .iter()
            .find(|r| r.starts_with('│'))
            .expect("an interior row");
        assert!(interior_row.matches('│').count() >= 2, "{interior_row:?}");
    }

    /// Each quadrant name is placed inside its own quarter of the grid.
    #[test]
    fn quadrant_names_are_placed_in_their_own_corners() {
        let rows = drawn(
            "quadrantChart\n\
             quadrant-1 TopRight\n\
             quadrant-2 TopLeft\n\
             quadrant-3 BottomLeft\n\
             quadrant-4 BottomRight\n\
             P: [0.5, 0.5]",
        );
        let text = rows.join("\n");
        for name in ["TopRight", "TopLeft", "BottomLeft", "BottomRight"] {
            assert!(text.contains(name), "{name} missing from:\n{text}");
        }
        let top_left_row = rows.iter().position(|r| r.contains("TopLeft")).unwrap();
        let top_right_row = rows.iter().position(|r| r.contains("TopRight")).unwrap();
        let bottom_left_row = rows.iter().position(|r| r.contains("BottomLeft")).unwrap();

        assert!(top_left_row < bottom_left_row);
        assert!(top_right_row < bottom_left_row);

        let left_col = rows[top_left_row].find("TopLeft").unwrap();
        let right_col = rows[top_right_row].find("TopRight").unwrap();
        assert!(left_col < right_col, "{left_col} vs {right_col}");
    }

    /// A point is plotted at its normalised position and labelled beside it.
    #[test]
    fn a_point_is_plotted_and_labelled() {
        let rows = drawn("quadrantChart\n  Campaign A: [0.1, 0.9]");
        let text = rows.join("\n");
        assert!(text.contains('●'), "{text}");
        assert!(text.contains("Campaign A"), "{text}");

        let marker_row = rows.iter().position(|r| r.contains('●')).unwrap();
        assert!(
            marker_row < rows.len() / 2,
            "{marker_row} of {}",
            rows.len()
        );
    }

    #[test]
    fn a_point_on_the_divider_is_nudged_off_it_leaving_the_cross_intact() {
        let rows = drawn("quadrantChart\n  P: [0.5, 0.5]");
        assert_eq!(
            rows,
            vec![
                "┌────────────────────┬────────────────────┐",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │● P                 │",
                "├────────────────────┼────────────────────┤",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │                    │",
                "│                    │                    │",
                "└────────────────────┴────────────────────┘",
            ]
        );
    }

    #[test]
    fn a_bare_axis_name_with_no_arrow_still_shows_a_label() {
        let rows = drawn("quadrantChart\n  x-axis Reach\n  P: [0.5, 0.5]");
        assert!(rows.iter().any(|r| r.contains("Reach")), "{rows:?}");
    }

    #[test]
    fn what_is_not_a_quadrant_chart_is_left_alone() {
        assert!(render_quadrant("graph TD\n A --> B").is_none());
        assert!(
            render_quadrant("quadrantChart").is_none(),
            "no points at all"
        );
        assert!(render_quadrant("quadrantChart\n  Bad point with no brackets: 0.3, 0.6").is_none(),);
        assert!(
            render_quadrant("quadrantChart\n  Bad: [nonsense, 0.6]").is_none(),
            "a point coordinate that does not parse as a number"
        );
    }

    #[test]
    fn too_many_points_are_refused() {
        let mut source = String::from("quadrantChart\n");
        for index in 0..200 {
            source.push_str(&format!("Point {index}: [0.5, 0.5]\n"));
        }
        assert!(render_quadrant(&source).is_none());
    }
}
