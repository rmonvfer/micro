//! End-to-end tests against the public API for sankey diagrams.

use micro_mermaid::{diagram_kind, render, DiagramKind};

fn plain(src: &str) -> Vec<String> {
    render(src)
        .unwrap_or_else(|| panic!("expected {src:?} to render"))
        .plain
}

#[test]
fn diagram_kind_recognises_a_sankey_header() {
    assert_eq!(
        diagram_kind("sankey-beta\nA,B,1"),
        Some(DiagramKind::Sankey)
    );
}

/// Flows are grouped under the node they leave, and each node carries the total leaving
/// it — what comes into a stage against what leaves it is what a sankey is read for.
#[test]
fn two_flows_from_one_source_are_grouped_under_it() {
    let rows = plain("sankey-beta\nSalary,Housing,20\nSalary,Food,10");
    assert_eq!(rows.len(), 3, "one heading and two flows: {rows:?}");
    assert!(rows[0].starts_with("Salary (30)"), "{rows:?}");
    assert!(rows[1].contains("→ Housing"), "{rows:?}");
    assert!(rows[1].trim_end().ends_with("20"), "{rows:?}");
    assert!(rows[2].contains("→ Food"), "{rows:?}");
}

/// A chain reads down the page, each stage heading its own flows.
#[test]
fn a_chain_reads_stage_by_stage() {
    let rows = plain("sankey-beta\nA,B,4\nB,C,4");
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(rows[0].starts_with("A (4)"), "{rows:?}");
    assert!(rows[1].contains("→ B"), "{rows:?}");
    assert!(rows[2].starts_with("B (4)"), "{rows:?}");
    assert!(rows[3].contains("→ C"), "{rows:?}");
}

/// The largest flow fills the bar and the rest are drawn against it, so which path carries
/// the volume is the first thing seen.
#[test]
fn bars_are_drawn_in_proportion() {
    let rows = plain("sankey-beta\nA,Big,100\nA,Small,25");
    let big = rows[1].matches('█').count();
    let small = rows[2].matches('█').count();
    assert!(big > small, "{rows:?}");
    assert_eq!(
        small * 4,
        big,
        "a quarter of the volume, a quarter of the bar"
    );
}

/// A source that is not a sankey is refused, and the caller shows the source instead.
#[test]
fn a_malformed_sankey_is_refused() {
    assert!(render("sankey-beta\nA,B").is_none(), "no value");
    assert!(render("sankey-beta\nA,B,nope").is_none(), "not a number");
    assert!(render("sankey-beta").is_none(), "nothing flows");
}
