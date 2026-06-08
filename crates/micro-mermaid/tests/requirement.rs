//! Requirement diagrams, exercised through the public `render` entry point the way
//! `tests/render.rs` exercises every other kind.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src).expect("drawn").plain
}

/// A requirement block draws as a bordered box carrying its stereotype, name and fields, corner to
/// corner.
#[test]
fn a_requirement_draws_a_bordered_box_with_its_fields() {
    let src = "requirementDiagram\nrequirement test_req {\nid: 1\n}";
    assert_eq!(
        plain(src),
        vec![
            " ┌───────────────┐",
            " │ «requirement» │",
            " │   test_req    │",
            " ├───────────────┤",
            " │ id: 1         │",
            " └───────────────┘",
        ]
    );
}

/// A relation between a requirement and an element is drawn as a routed, labelled arrow.
#[test]
fn a_relation_connects_two_boxes_with_a_routed_labelled_arrow() {
    let src = "requirementDiagram\n\
               requirement test_req {\n\
               id: 1\n\
               text: the test text.\n\
               risk: high\n\
               verifymethod: test\n\
               }\n\
               element test_entity {\n\
               type: simulation\n\
               }\n\
               test_entity - satisfies -> test_req";
    let rows = plain(src);
    let text = rows.join("\n");
    assert!(text.contains("«requirement»"), "{text}");
    assert!(text.contains("«element»"), "{text}");
    assert!(text.contains("«satisfies»"), "{text}");
    assert!(
        text.contains('│') || text.contains('╎'),
        "a routed line: {text}"
    );
}

#[test]
fn an_unparseable_requirement_diagram_renders_nothing() {
    let src = "requirementDiagram\nrequirement r {\nid 1\n}";
    assert!(render(src).is_none());

    assert!(diagram_kind(src).is_some());
}
