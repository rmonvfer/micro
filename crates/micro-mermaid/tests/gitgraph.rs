//! End-to-end tests against the public API for git graphs.

use micro_mermaid::{diagram_kind, render, DiagramKind};

fn plain(src: &str) -> Vec<String> {
    render(src)
        .unwrap_or_else(|| panic!("expected {src:?} to render"))
        .plain
}

#[test]
fn diagram_kind_recognises_a_gitgraph_header() {
    assert!(diagram_kind("gitGraph\n  commit").is_some());
}

#[test]
fn commits_are_drawn_as_marks_on_the_main_lane() {
    let rows = plain("gitGraph\n  commit\n  commit\n  commit");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].starts_with("main "), "{rows:?}");
    assert_eq!(rows[0].matches('●').count(), 3);
}

#[test]
fn a_feature_branch_merges_back_into_main() {
    let src = "gitGraph\n  commit\n  branch feature\n  commit\n  checkout main\n  merge feature";
    let rows = plain(src);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].starts_with("main "), "{rows:?}");
    assert!(rows[1].starts_with("feature "), "{rows:?}");
    assert_eq!(
        rows[0].matches('●').count(),
        2,
        "the fork commit and the merge commit"
    );
}

#[test]
fn a_tag_and_a_highlighted_commit_are_both_visible() {
    let rows = plain("gitGraph\n  commit tag: \"v1.0\"\n  commit type: HIGHLIGHT");
    let joined = rows.join("\n");
    assert!(joined.contains("(v1.0)"), "{rows:?}");
    assert!(joined.contains('◆'), "{rows:?}");
}

#[test]
fn something_that_is_not_a_gitgraph_is_not_drawn_as_one() {
    assert_eq!(
        diagram_kind("flowchart LR\n  A --> B"),
        Some(DiagramKind::Flowchart)
    );
    assert!(render("gitGraph").is_none(), "no commits at all");
}
