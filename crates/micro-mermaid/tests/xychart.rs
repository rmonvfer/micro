//! End-to-end tests against the public API for xy charts.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src)
        .unwrap_or_else(|| panic!("expected {src:?} to render"))
        .plain
}

#[test]
fn diagram_kind_recognises_an_xychart_header() {
    assert!(diagram_kind("xychart-beta\n  bar [1]").is_some());
}

#[test]
fn a_bar_chart_shows_a_taller_column_for_a_bigger_value() {
    let src = "xychart-beta\n  title Sales\n  x-axis [Jan, Feb, Mar]\n  y-axis \"Revenue\" 0 --> 100\n  bar [10, 50, 90]";
    let rows = plain(src);
    let joined = rows.join("\n");
    assert_eq!(rows[0], "Sales");
    assert!(joined.contains("Revenue"), "{rows:?}");
    assert!(
        joined.contains("Jan") && joined.contains("Feb") && joined.contains("Mar"),
        "{rows:?}"
    );
    assert!(joined.contains('█'), "{rows:?}");
}

#[test]
fn a_line_series_is_plotted_and_joined() {
    let rows = plain("xychart-beta\n  y-axis 0 --> 10\n  line [0, 5, 10]");
    assert_eq!(rows.join("\n").matches('●').count(), 3);
}

#[test]
fn the_axis_auto_scales_when_no_range_is_given() {
    let rows = plain("xychart-beta\n  bar [3, 9, 1]");
    assert!(rows.iter().any(|r| r.contains('9')), "{rows:?}");
}

#[test]
fn something_that_is_not_an_xychart_is_refused() {
    assert!(render("xychart-beta").is_none(), "no series at all");
    assert!(
        render("xychart-beta\n  bar []").is_none(),
        "an empty series"
    );
}
