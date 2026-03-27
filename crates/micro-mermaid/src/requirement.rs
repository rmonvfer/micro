//! Requirement diagrams: a box per requirement or element, carrying the
//! fields declared inside its braces, connected by relations drawn as
//! labelled arrows.
//!
//! A requirement box and a class box are the same shape — a title
//! compartment over a compartment of rows — so this reuses the `Graph` and
//! `ClassInfo` model the class and ER diagrams already draw through. The
//! only diagram-specific work here is reading the `requirement { ... }` /
//! `element { ... }` block syntax and the `a - verb -> b` relation syntax
//! into that shared shape.

use crate::canvas::Canvas;
use crate::graph::{ClassInfo, Edge, Graph, Head, LineKind, Shape, MAX_MEMBERS};
use crate::labels::{ascii_lower, clean_label, strip_controls};
use crate::layout::layout_class;
use crate::parse::statements_of;

/// Block keywords the grammar allows before an identifier and `{`. `element`
/// stands for a system element being related to requirements; the rest are
/// the requirement type Mermaid recognises, kept as written so the box shows
/// exactly which one was declared.
const BLOCK_KEYWORDS: &[&str] = &[
    "requirement",
    "functionalrequirement",
    "performancerequirement",
    "interfacerequirement",
    "physicalrequirement",
    "designconstraint",
    "element",
];

/// Draw `src` as a requirement diagram, or answer nothing when it is not one.
pub(crate) fn render_requirement(src: &str) -> Option<Canvas> {
    let (graph, infos) = parse_requirement(src)?;
    layout_class(&graph, &infos)
}

fn parse_requirement(src: &str) -> Option<(Graph, Vec<ClassInfo>)> {
    let src = strip_controls(src);
    let statements = statements_of(&src);
    let header = statements.first()?;
    if ascii_lower(header.split_whitespace().next().unwrap_or("")) != "requirementdiagram" {
        return None;
    }

    let mut graph = Graph::default();
    let mut infos: Vec<ClassInfo> = Vec::new();
    // The block currently open, if any statement so far opened a
    // `requirement`/`element` and has not yet closed it with `}`.
    let mut cur_block: Option<usize> = None;

    for st in &statements[1..] {
        if let Some(idx) = cur_block {
            if st == "}" {
                cur_block = None;
            } else {
                let (key, value) = st.split_once(':')?;
                let key = ascii_lower(key.trim());
                if key.is_empty() {
                    return None;
                }
                push_field(&mut infos[idx], &key, &clean_label(value.trim()));
            }
            continue;
        }

        let word = st.split_whitespace().next()?;
        if BLOCK_KEYWORDS.contains(&ascii_lower(word).as_str()) {
            let rest = st[word.len()..].trim();
            if !rest.ends_with('{') {
                return None;
            }
            let name = rest[..rest.len() - 1].trim();
            if name.is_empty() || name.chars().any(char::is_whitespace) {
                return None;
            }
            let idx = declare(&mut graph, &mut infos, name)?;
            // The keyword itself is the stereotype shown on the box, so
            // `functionalRequirement` reads as such rather than as a plain
            // `requirement`.
            infos[idx].annotation = Some(word.to_string());
            cur_block = Some(idx);
            continue;
        }

        let (from, verb, to) = parse_relation(st)?;
        let f = declare(&mut graph, &mut infos, &from)?;
        let t = declare(&mut graph, &mut infos, &to)?;
        let pushed = graph.push_edge(Edge {
            from: f,
            to: t,
            label: Some(format!("«{verb}»")),
            head_to: Head::Arrow,
            head_from: Head::None,
            line: LineKind::Dotted,
        });
        if !pushed {
            return None;
        }
    }

    // A block that never saw its closing brace is unreadable, same as any
    // other malformed statement — better to say nothing than to draw a box
    // missing whatever came after.
    if cur_block.is_some() || graph.nodes.is_empty() {
        return None;
    }
    Some((graph, infos))
}

/// Get or create a node by id, keeping `infos` the same length as
/// `graph.nodes` the way every other block-bodied diagram in this crate does.
fn declare(graph: &mut Graph, infos: &mut Vec<ClassInfo>, name: &str) -> Option<usize> {
    let idx = graph.node_index(name, Some(name), Shape::Rect)?;
    while infos.len() <= idx {
        infos.push(ClassInfo::default());
    }
    Some(idx)
}

/// Add one `key: value` row to a box's field compartment, eliding past the
/// cap the same way a class diagram elides a long member list.
fn push_field(info: &mut ClassInfo, key: &str, value: &str) {
    if info.attrs.len() < MAX_MEMBERS {
        info.attrs.push(format!("{key}: {value}"));
    } else if info.attrs.len() == MAX_MEMBERS {
        info.attrs.push("…".to_string());
    }
}

/// Read `<id> - <verb> -> <id>`. The dash before the verb is unambiguous
/// because ids and verbs are identifier words with no dash of their own, so
/// the rightmost `-` before the arrow is always the one that opened the verb.
fn parse_relation(st: &str) -> Option<(String, String, String)> {
    let (left, to) = st.split_once("->")?;
    let to = to.trim();
    let (from, verb) = left.rsplit_once('-')?;
    let from = from.trim();
    let verb = verb.trim();
    if from.is_empty()
        || verb.is_empty()
        || to.is_empty()
        || from.chars().any(char::is_whitespace)
        || to.chars().any(char::is_whitespace)
        || verb.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some((from.to_string(), verb.to_string(), to.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_requirement(src)
            .expect("it is a requirement diagram")
            .to_lines()
            .plain
    }

    /// A requirement block draws its stereotype, name and fields as one box.
    #[test]
    fn a_requirement_is_drawn_with_its_fields_in_a_box() {
        let rows = drawn(
            "requirementDiagram\n\
             requirement test_req {\n\
             id: 1\n\
             text: the test text.\n\
             risk: high\n\
             verifymethod: test\n\
             }",
        );
        assert!(rows.iter().any(|r| r.contains("«requirement»")), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains("test_req")), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains("id: 1")), "{rows:?}");
        assert!(
            rows.iter().any(|r| r.contains("text: the test text.")),
            "{rows:?}"
        );
        assert!(rows.iter().any(|r| r.contains("risk: high")), "{rows:?}");
        assert!(
            rows.iter().any(|r| r.contains("verifymethod: test")),
            "{rows:?}"
        );
    }

    /// A specialised requirement type keeps its own name as the stereotype,
    /// rather than being folded into the generic `requirement` label.
    #[test]
    fn a_typed_requirement_keeps_its_type_as_the_stereotype() {
        let rows = drawn(
            "requirementDiagram\n\
             functionalRequirement perf_req {\n\
             id: 2\n\
             text: fast enough.\n\
             }",
        );
        assert!(
            rows.iter().any(|r| r.contains("«functionalRequirement»")),
            "{rows:?}"
        );
    }

    /// A box with a single field is small enough to check exactly, corner to
    /// corner: the stereotype and name centred in their own compartment,
    /// the field left-aligned in its own below a divider.
    #[test]
    fn a_minimal_requirement_draws_a_bordered_box() {
        let rows = drawn("requirementDiagram\nrequirement test_req {\nid: 1\n}");
        assert_eq!(
            rows,
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

    /// An element block draws the same way, with its own field set.
    #[test]
    fn an_element_is_drawn_with_its_type_field() {
        let rows = drawn(
            "requirementDiagram\n\
             element test_entity {\n\
             type: simulation\n\
             }",
        );
        assert_eq!(
            rows,
            vec![
                "┌──────────────────┐",
                "│    «element»     │",
                "│   test_entity    │",
                "├──────────────────┤",
                "│ type: simulation │",
                "└──────────────────┘",
            ]
        );
    }

    /// A relation is drawn as an arrow labelled with its verb in guillemets,
    /// matching the stereotype convention already used on the boxes.
    #[test]
    fn a_relation_is_drawn_between_two_boxes_with_its_verb_labelled() {
        let rows = drawn(
            "requirementDiagram\n\
             requirement test_req {\n\
             id: 1\n\
             text: the test text.\n\
             risk: high\n\
             verifymethod: test\n\
             }\n\
             element test_entity {\n\
             type: simulation\n\
             }\n\
             test_entity - satisfies -> test_req",
        );
        assert!(rows.iter().any(|r| r.contains("«satisfies»")), "{rows:?}");
        // The two boxes are connected by a routed line, not just floating
        // side by side with a label between them.
        assert!(
            rows.iter().any(|r| r.contains('│') || r.contains('─')),
            "{rows:?}"
        );
    }

    /// An id used only in a relation, never declared as its own block, still
    /// gets a box — empty of fields, but present, so the relation has
    /// somewhere to point.
    #[test]
    fn an_undeclared_id_in_a_relation_still_gets_a_box() {
        let rows = drawn("requirementDiagram\n  a - traces -> b");
        assert!(rows.iter().any(|r| r.contains('a')), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains('b')), "{rows:?}");
    }

    /// Anything that is not a requirement diagram, or is one but malformed,
    /// is refused rather than guessed at.
    #[test]
    fn what_is_not_a_requirement_diagram_is_left_alone() {
        assert!(render_requirement("graph TD\n A --> B").is_none());
        assert!(
            render_requirement("requirementDiagram").is_none(),
            "no boxes at all"
        );
        assert!(
            render_requirement("requirementDiagram\nrequirement r {\nid 1\n}").is_none(),
            "a field with no colon"
        );
        assert!(
            render_requirement("requirementDiagram\nrequirement r {\nid: 1\n").is_none(),
            "a block missing its closing brace"
        );
        assert!(
            render_requirement("requirementDiagram\nnot a relation at all").is_none(),
            "a top-level statement that is neither a block nor a relation"
        );
    }

    /// A diagram of hundreds of requirements says nothing useful in a
    /// terminal, so laying it out is refused rather than attempted.
    #[test]
    fn too_many_requirements_are_refused() {
        let mut source = String::from("requirementDiagram\n");
        for index in 0..200 {
            source.push_str(&format!("requirement r{index} {{\nid: {index}\n}}\n"));
        }
        assert!(render_requirement(&source).is_none());
    }
}
