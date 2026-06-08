//! Architecture diagrams: services inside their group frames, connected by plain lines.

use std::collections::HashMap;

use crate::canvas::Canvas;
use crate::graph::{Edge, Graph, Group, Head, LineKind, Shape, MAX_GROUPS};
use crate::labels::{ascii_lower, clean_label, is_id_char, strip_controls};
use crate::layout::{layout_flowchart, layout_grouped};
use crate::parse::statements_of;

/// Draw `src` as an architecture diagram, or answer nothing when it is not one.
pub(crate) fn render_architecture(src: &str) -> Option<Canvas> {
    let graph = parse_architecture(src)?;
    if graph.groups.is_empty() {
        layout_flowchart(&graph)
    } else {
        layout_grouped(&graph)
    }
}

fn parse_architecture(src: &str) -> Option<Graph> {
    let src = strip_controls(src);
    let statements = statements_of(&src);
    let header = statements.first()?;
    if ascii_lower(header.split_whitespace().next().unwrap_or("")) != "architecture-beta" {
        return None;
    }

    let mut graph = Graph::default();
    let mut group_index: HashMap<String, usize> = HashMap::new();

    for st in &statements[1..] {
        let word = st.split_whitespace().next()?;
        let rest = st[word.len()..].trim();
        match ascii_lower(word).as_str() {
            "group" => {
                let (id, icon, title, suffix) = parse_decl(rest)?;
                let parent = read_in_suffix(suffix, &group_index)?;
                if graph.groups.len() >= MAX_GROUPS {
                    return None;
                }
                let index = graph.groups.len();
                let label = labelled(title, icon).unwrap_or_else(|| id.clone());
                group_index.insert(id.clone(), index);
                graph.groups.push(Group { id, label, parent });
            }
            "service" => {
                let (id, icon, title, suffix) = parse_decl(rest)?;
                graph.cur_group = read_in_suffix(suffix, &group_index)?;
                let label = labelled(title, icon);
                graph.node_index(&id, label.as_deref(), Shape::Rect)?;
                graph.cur_group = None;
            }
            _ => {
                let (from, to) = parse_connection(st)?;
                let f = graph.node_index(&from, None, Shape::Rect)?;
                let t = graph.node_index(&to, None, Shape::Rect)?;
                let pushed = graph.push_edge(Edge {
                    from: f,
                    to: t,
                    label: None,
                    head_to: Head::None,
                    head_from: Head::None,
                    line: LineKind::Solid,
                });
                if !pushed {
                    return None;
                }
            }
        }
    }

    if graph.nodes.is_empty() {
        return None;
    }
    Some(graph)
}

/// `title`, folded with its `«icon»` when both are present.
fn labelled(title: Option<String>, icon: Option<String>) -> Option<String> {
    match (title, icon) {
        (Some(t), Some(i)) => Some(format!("{t} «{i}»")),
        (Some(t), None) => Some(t),
        (None, Some(i)) => Some(format!("«{i}»")),
        (None, None) => None,
    }
}

/// `id(icon)[Title]`, none of which but `id` is required.
fn parse_decl(s: &str) -> Option<(String, Option<String>, Option<String>, &str)> {
    let s = s.trim_start();
    let id_end = s
        .find(|c: char| c == '(' || c == '[' || c.is_whitespace())
        .unwrap_or(s.len());
    let id = s[..id_end].trim().to_string();
    if id.is_empty() || !id.chars().all(is_id_char) {
        return None;
    }

    let mut rest = s[id_end..].trim_start();
    let mut icon = None;
    if let Some(after_paren) = rest.strip_prefix('(') {
        let (inside, after) = after_paren.split_once(')')?;
        let inside = inside.trim();
        if !inside.is_empty() {
            icon = Some(inside.to_string());
        }
        rest = after.trim_start();
    }

    let mut title = None;
    if let Some(after_bracket) = rest.strip_prefix('[') {
        let (inside, after) = after_bracket.split_once(']')?;
        title = Some(clean_label(inside));
        rest = after;
    }

    Some((id, icon, title, rest.trim()))
}

/// The remainder of a `group`/`service` line after its declaration: empty, or `in <parent id>`.
fn read_in_suffix(suffix: &str, group_index: &HashMap<String, usize>) -> Option<Option<usize>> {
    if suffix.is_empty() {
        return Some(None);
    }
    let mut words = suffix.split_whitespace();
    if ascii_lower(words.next()?) != "in" {
        return None;
    }
    let parent_id = words.next()?;
    if words.next().is_some() {
        return None;
    }
    Some(Some(*group_index.get(parent_id)?))
}

/// `a:R -- L:b`, or the bare `a -- b` the same grammar also allows: the port letter sits beside
/// whichever id it belongs to.
fn parse_connection(st: &str) -> Option<(String, String)> {
    let (left, right) = st.split_once("--")?;
    let from = left.trim().split(':').next()?.trim();
    let to = right.trim().rsplit(':').next()?.trim();
    if from.is_empty()
        || to.is_empty()
        || from.chars().any(char::is_whitespace)
        || to.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some((from.to_string(), to.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_architecture(src)
            .expect("it is an architecture diagram")
            .to_lines()
            .plain
    }

    /// A service inside a group is drawn inside that group's frame.
    #[test]
    fn a_service_is_drawn_inside_its_group_frame() {
        let rows = drawn(
            "architecture-beta\n\
             group api(cloud)[API]\n\
             service db(database)[Database] in api",
        );
        let text = rows.join("\n");
        assert!(text.contains("API"), "{text}");
        assert!(text.contains("Database"), "{text}");
        assert!(
            text.contains("«database»"),
            "the icon reads as a stereotype: {text}"
        );

        assert_eq!(text.matches('┌').count(), 2, "{text}");
    }

    #[test]
    fn a_connection_draws_a_line_between_two_services() {
        let rows = drawn(
            "architecture-beta\n\
             service db(database)[Database]\n\
             service srv(server)[Server]\n\
             db:R -- L:srv",
        );
        let text = rows.join("\n");
        assert!(
            text.contains("Database") && text.contains("Server"),
            "{text}"
        );
        assert!(text.contains('─') || text.contains('│'), "{text}");
    }

    /// A service with no group at all still draws, plainly, the way a flowchart with no subgraph
    /// does.
    #[test]
    fn a_service_with_no_group_still_draws() {
        let rows = drawn("architecture-beta\n  service solo(server)[Solo]");
        assert!(rows.iter().any(|r| r.contains("Solo")), "{rows:?}");
    }

    #[test]
    fn what_is_not_an_architecture_diagram_is_left_alone() {
        assert!(render_architecture("graph TD\n A --> B").is_none());
        assert!(
            render_architecture("architecture-beta").is_none(),
            "nothing declared"
        );
        assert!(
            render_architecture("architecture-beta\n  service db(database)[Database] in nowhere")
                .is_none(),
            "a group that was never declared"
        );
    }
}
