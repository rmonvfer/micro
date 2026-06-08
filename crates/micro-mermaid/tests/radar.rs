//! Radar charts, exercised through the public `render` entry point the way `tests/render.rs`
//! exercises every other kind.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src).expect("drawn").plain
}

/// Each axis is a row with every curve's value marked on a shared scale, and a legend names which
/// glyph belongs to which curve.
#[test]
fn each_axis_is_a_row_with_a_legend_naming_the_glyphs() {
    let src = "radar-beta\n\
               title Skills\n\
               axis a[\"Communication\"], b[\"Technical\"]\n\
               curve name1[\"Alice\"]{1,5}\n\
               curve name2[\"Bob\"]{5,1}\n\
               max 5\n\
               min 0";
    let rows = plain(src);
    assert_eq!(rows[0], "Skills");
    let text = rows.join("\n");
    assert!(
        text.contains("Communication") && text.contains("Technical"),
        "{text}"
    );
    let legend = rows.last().unwrap();
    assert!(
        legend.contains('●') && legend.contains("Alice"),
        "{legend:?}"
    );
    assert!(legend.contains('○') && legend.contains("Bob"), "{legend:?}");
}

#[test]
fn an_unparseable_radar_chart_renders_nothing() {
    let src = "radar-beta\n  axis a[\"A\"]";
    assert!(render(src).is_none());
    assert!(diagram_kind(src).is_some());
}
