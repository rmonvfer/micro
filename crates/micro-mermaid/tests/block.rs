//! End-to-end tests against the public API for block diagrams.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src)
        .unwrap_or_else(|| panic!("expected {src:?} to render"))
        .plain
}

#[test]
fn diagram_kind_recognises_a_block_beta_header() {
    assert!(diagram_kind("block-beta\n  a").is_some());
}

#[test]
fn columns_lays_blocks_into_a_grid() {
    let rows = plain("block-beta\n  columns 2\n  a[\"Alpha\"]\n  b[\"Beta\"]\n  c[\"Gamma\"]");
    let joined = rows.join("\n");
    assert!(
        joined.contains("Alpha") && joined.contains("Beta") && joined.contains("Gamma"),
        "{rows:?}"
    );
    let row_of = |s: &str| rows.iter().position(|r| r.contains(s)).unwrap();
    assert!(
        row_of("Gamma") > row_of("Alpha"),
        "third block wraps to a new row: {rows:?}"
    );
}

#[test]
fn a_group_frames_its_own_blocks() {
    let rows = plain("block-beta\n  block:services[\"Services\"]\n    api\n    db\n  end");
    let joined = rows.join("\n");
    assert!(joined.contains("Services"), "{rows:?}");
    assert!(joined.contains("api") && joined.contains("db"), "{rows:?}");
    assert!(
        joined.contains('┌') && joined.contains('┘'),
        "framed: {rows:?}"
    );
}

#[test]
fn an_arrow_connects_two_blocks_with_an_arrowhead() {
    let rows = plain("block-beta\n  columns 1\n  a\n  b\n  a --> b");
    assert!(rows.join("\n").contains('▼'), "{rows:?}");
}

#[test]
fn something_that_is_not_a_block_diagram_is_refused() {
    assert!(render("block-beta").is_none(), "no blocks at all");
    assert!(
        render("block-beta\n  block:grp\n    a").is_none(),
        "an unclosed group"
    );
}
