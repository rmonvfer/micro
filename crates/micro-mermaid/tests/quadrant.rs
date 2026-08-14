//! Quadrant charts, exercised through the public `render` entry point the
//! way `tests/render.rs` exercises every other kind.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src).expect("drawn").plain
}

/// The grid is one continuous bordered box split into four by a cross, and
/// a point at the exact centre is nudged off the cross rather than drawn on
/// top of it — every border and junction glyph, corner to corner.
#[test]
fn the_grid_is_a_bordered_box_with_a_point_nudged_off_its_centre() {
    let src = "quadrantChart\n  P: [0.5, 0.5]";
    assert_eq!(
        plain(src),
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

/// Quadrant names, axis range labels and a plotted point all show up
/// together in one rendered chart.
#[test]
fn quadrant_names_axes_and_points_are_all_drawn_together() {
    let src = "quadrantChart\n\
               title Campaigns\n\
               x-axis Low Reach --> High Reach\n\
               y-axis Low Engagement --> High Engagement\n\
               quadrant-1 Expand\n\
               quadrant-2 Promote\n\
               quadrant-3 Re-evaluate\n\
               quadrant-4 Improve\n\
               Campaign A: [0.8, 0.8]";
    let rows = plain(src);
    let text = rows.join("\n");
    assert_eq!(rows[0], "Campaigns");
    // "Campaign A" itself may be truncated with an ellipsis if the point
    // lands somewhere tight, so only a prefix short enough to survive that
    // is checked for.
    for word in [
        "Expand",
        "Promote",
        "Re-evaluate",
        "Improve",
        "Campa",
        "Reach",
        "Engagement",
    ] {
        assert!(text.contains(word), "{word} missing from:\n{text}");
    }
}

/// An unparseable quadrant chart renders nothing, the same way an
/// unparseable class diagram does.
#[test]
fn an_unparseable_quadrant_chart_renders_nothing() {
    let src = "quadrantChart\n  Bad: [nonsense, 0.6]";
    assert!(render(src).is_none());
    // `diagram_kind` still recognises the header, which is what tells a
    // syntax error apart from a diagram type this renderer does not draw.
    assert!(diagram_kind(src).is_some());
}
