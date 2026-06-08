use crate::canvas::Canvas;
use crate::graph::{ClassInfo, Edge, Graph, Head, LineKind, Shape, MAX_MEMBERS};
use crate::labels::{ascii_lower, clean_label, strip_controls};
use crate::layout::layout_class;
use crate::parse::statements_of;

/// Block keywords the grammar allows before an identifier and `{`.
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

    if cur_block.is_some() || graph.nodes.is_empty() {
        return None;
    }
    Some((graph, infos))
}

fn declare(graph: &mut Graph, infos: &mut Vec<ClassInfo>, name: &str) -> Option<usize> {
    let idx = graph.node_index(name, Some(name), Shape::Rect)?;
    while infos.len() <= idx {
        infos.push(ClassInfo::default());
    }
    Some(idx)
}

fn push_field(info: &mut ClassInfo, key: &str, value: &str) {
    if info.attrs.len() < MAX_MEMBERS {
        info.attrs.push(format!("{key}: {value}"));
    } else if info.attrs.len() == MAX_MEMBERS {
        info.attrs.push("…".to_string());
    }
}

/// Read `<id> - <verb> -> <id>`.
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

        assert!(
            rows.iter().any(|r| r.contains('│') || r.contains('─')),
            "{rows:?}"
        );
    }

    /// An id used only in a relation, never declared as its own block, still gets a box.
    #[test]
    fn an_undeclared_id_in_a_relation_still_gets_a_box() {
        let rows = drawn("requirementDiagram\n  a - traces -> b");
        assert!(rows.iter().any(|r| r.contains('a')), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains('b')), "{rows:?}");
    }

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

    #[test]
    fn too_many_requirements_are_refused() {
        let mut source = String::from("requirementDiagram\n");
        for index in 0..200 {
            source.push_str(&format!("requirement r{index} {{\nid: {index}\n}}\n"));
        }
        assert!(render_requirement(&source).is_none());
    }
}
