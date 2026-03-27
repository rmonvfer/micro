//! Treemaps, exercised through the public `render` entry point the way
//! `tests/render.rs` exercises every other kind.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src).expect("drawn").plain
}

/// A leaf's value and its share of its parent are written beside it, and a
/// parent with no value of its own is worth the sum of its children.
#[test]
fn a_leaf_carries_its_value_and_a_parent_sums_its_children() {
    let src = "treemap-beta\ntitle Budget\n\"Category A\"\n  \"Item 1\": 10\n  \"Item 2\": 30";
    let rows = plain(src);
    assert_eq!(rows[0], "Budget");
    let text = rows.join("\n");
    assert!(
        text.contains("Item 1") && text.contains("(10, 25%)"),
        "{text}"
    );
    assert!(
        text.contains("Category A") && text.contains("(40, 100%)"),
        "{text}"
    );
}

/// An unparseable treemap renders nothing, the same way an unparseable
/// class diagram does.
#[test]
fn an_unparseable_treemap_renders_nothing() {
    let src = "treemap-beta\n\"\"";
    assert!(render(src).is_none());
    assert!(diagram_kind(src).is_some());
}
