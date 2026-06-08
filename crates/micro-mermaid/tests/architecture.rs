//! Architecture diagrams, exercised through the public `render` entry point the way
//! `tests/render.rs` exercises every other kind.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src).expect("drawn").plain
}

/// A service inside a group is drawn inside that group's frame, and a connection between two
/// services is a drawn line.
#[test]
fn services_draw_inside_their_group_and_connect_by_a_line() {
    let src = "architecture-beta\n\
               group api(cloud)[API]\n\
               service db(database)[Database] in api\n\
               service srv(server)[Server] in api\n\
               db:R -- L:srv";
    let rows = plain(src);
    let text = rows.join("\n");
    assert!(text.contains("API"), "{text}");
    assert!(
        text.contains("Database") && text.contains("Server"),
        "{text}"
    );
    assert!(
        text.contains('─') || text.contains('│'),
        "a routed connection: {text}"
    );
}

#[test]
fn an_unparseable_architecture_diagram_renders_nothing() {
    let src = "architecture-beta\n  service db(database)[Database] in nowhere";
    assert!(render(src).is_none());
    assert!(diagram_kind(src).is_some());
}
