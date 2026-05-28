//! Kanban boards, exercised through the public `render` entry point the way `tests/render.rs`
//! exercises every other kind.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src).expect("drawn").plain
}

/// Columns of cards are drawn side by side, each task its own bordered card under its column's
/// heading.
#[test]
fn columns_of_cards_are_drawn_side_by_side() {
    let src = "kanban\n  todo[To Do]\n    t1[Write docs]\n  done[Done]\n    t2[Ship it]@{ assigned: 'Alice', priority: 'High' }";
    let rows = plain(src);
    let text = rows.join("\n");
    assert!(text.contains("To Do") && text.contains("Done"), "{text}");
    assert!(text.contains("Write docs"), "{text}");
    assert!(text.contains("High · Alice"), "{text}");
    assert!(
        rows.iter().any(|r| r.contains('┌')) && rows.iter().any(|r| r.contains('│')),
        "cards are fully bordered, sides included: {rows:?}"
    );
}


#[test]
fn an_unparseable_kanban_board_renders_nothing() {
    let src = "kanban\n  [Column with no id]";
    assert!(render(src).is_none());
    assert!(diagram_kind(src).is_some());
}
