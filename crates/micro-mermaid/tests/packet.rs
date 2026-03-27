//! Packet diagrams, exercised through the public `render` entry point the
//! way `tests/render.rs` exercises every other kind.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src).expect("drawn").plain
}

/// Two adjacent fields share the border between them, with the bit numbers
/// for the whole row along the top.
#[test]
fn adjacent_fields_share_the_border_between_them() {
    let src = "packet-beta\ntitle IP header\n0-15: \"Source Port\"\n16-31: \"Destination Port\"";
    let rows = plain(src);
    assert_eq!(rows[0], "IP header");
    let text = rows.join("\n");
    assert!(
        text.contains("Source Port") && text.contains("Destination Port"),
        "{text}"
    );
    let border = rows
        .iter()
        .find(|r| r.starts_with('┌'))
        .expect("a top border");
    assert!(
        border.contains('┬'),
        "the shared boundary tees into it: {border:?}"
    );
    assert!(
        border.ends_with('┐'),
        "the right edge closes it: {border:?}"
    );
}

/// An unparseable packet diagram renders nothing, the same way an
/// unparseable class diagram does.
#[test]
fn an_unparseable_packet_diagram_renders_nothing() {
    let src = "packet-beta\n0-15 no colon at all";
    assert!(render(src).is_none());
    assert!(diagram_kind(src).is_some());
}
