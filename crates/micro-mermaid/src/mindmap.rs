//! Mind maps, drawn as a tree lying on its side.
//!
//! A mind map is a root and what hangs off it, and depth in the source is written as
//! indentation. Drawn growing rightward, each branch keeps its own row and the connectors
//! between a parent and its children read down the left of them — which is how a reader
//! follows one branch without losing the others, and what a mind map is for.

use crate::canvas::draw_text;
use crate::canvas::Canvas;
use crate::labels::{clean_label, strip_controls};
use crate::types::Cls;
use crate::width::string_width;

/// Nodes past this and the map is refused, on the same grounds as every other cap here: a
/// diagram nobody can take in is not worth the room it would need.
const MAX_NODES: usize = 128;

/// Columns between one depth and the next, which is where the connectors are drawn.
const STEP: usize = 4;

struct Node {
    label: String,
    depth: usize,
    /// Which row this node's own text sits on, filled in once the rows are counted.
    row: usize,
}

/// Draw `src` as a mind map, or answer nothing when it is not one.
pub(crate) fn render_mindmap(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let mut lines = src.lines().filter(|line| !line.trim().is_empty());

    if lines.next()?.trim() != "mindmap" {
        return None;
    }

    // Indentation says what hangs off what, so it is measured before anything is trimmed.
    let mut nodes: Vec<Node> = Vec::new();
    let mut indents: Vec<usize> = Vec::new();
    for line in lines {
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        let label = read_label(line.trim())?;
        if nodes.len() >= MAX_NODES {
            return None;
        }

        // A line indented further than the last is a child of it; one indented the same or
        // less closes however many branches it takes to find its own parent.
        while indents.last().is_some_and(|last| *last >= indent) {
            indents.pop();
        }
        let depth = indents.len();
        indents.push(indent);

        nodes.push(Node {
            label,
            depth,
            row: nodes.len(),
        });
    }

    if nodes.len() < 2 {
        return None;
    }
    Some(draw(&nodes))
}

/// The words in a node, whatever brackets were used to say what shape it should be.
///
/// The shape is lost in a terminal — a cloud and a circle are the same handful of cells —
/// so only the words are kept, which is what the node was about.
fn read_label(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    // An id may carry its label in brackets: `root((text))`, `id[text]`, `id(text)`.
    for (open, close) in [
        ("((", "))"),
        ("))", "(("),
        ("[[", "]]"),
        ("[", "]"),
        ("(", ")"),
        ("{{", "}}"),
    ] {
        if let Some(start) = text.find(open) {
            let after = &text[start + open.len()..];
            if let Some(end) = after.find(close) {
                let label = clean_label(after[..end].trim());
                return match label.is_empty() {
                    true => None,
                    false => Some(label),
                };
            }
        }
    }

    let label = clean_label(text);
    (!label.is_empty()).then_some(label)
}

fn draw(nodes: &[Node]) -> Canvas {
    let width = nodes
        .iter()
        .map(|node| node.depth * STEP + string_width(&node.label))
        .max()
        .unwrap_or(0);
    let mut canvas = Canvas::new(width.max(1), nodes.len());

    for (index, node) in nodes.iter().enumerate() {
        let x = node.depth * STEP;
        draw_text(&mut canvas, &node.label, x, node.row, Cls::Text);

        if node.depth == 0 {
            continue;
        }

        // The branch is drawn back to whichever node above holds this one, and down the
        // column between them: a child says which parent it belongs to by the line it
        // hangs from, so the line has to reach that parent and no further.
        let parent = nodes[..index]
            .iter()
            .rposition(|above| above.depth < node.depth)
            .unwrap_or(0);
        let column = x - STEP + 1;

        // Whether anything else hangs off the same parent below this, which is what says
        // the branch carries on past this row rather than ending at it.
        let more_below = nodes[index + 1..]
            .iter()
            .take_while(|below| below.depth >= node.depth)
            .any(|below| below.depth == node.depth);

        // The rows between belong to earlier siblings and their children; the branch runs
        // down through them, but never over a corner already turned.
        for row in nodes[parent].row + 1..node.row {
            if canvas.ch[canvas.idx(column, row)] == " " {
                draw_text(&mut canvas, "│", column, row, Cls::Edge);
            }
        }
        // The corner turns into the node, and says whether the branch goes on below it.
        let corner = match more_below {
            true => "├",
            false => "└",
        };
        draw_text(&mut canvas, corner, column, node.row, Cls::Edge);
        for cell in column + 1..x {
            draw_text(&mut canvas, "─", cell, node.row, Cls::Edge);
        }
    }

    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_mindmap(src)
            .expect("it is a mind map")
            .to_lines()
            .plain
    }

    /// Depth is indentation, and each branch hangs from the node above it.
    #[test]
    fn a_child_hangs_from_the_node_it_belongs_to() {
        let rows = drawn("mindmap\n  root((Ideas))\n    First\n    Second");
        assert_eq!(rows[0], "Ideas");
        assert!(rows[1].contains("First"), "{rows:?}");
        assert!(
            rows[1].contains('├'),
            "hangs from the root, with more below: {rows:?}"
        );
        assert!(
            rows[2].contains('└'),
            "and the last one closes it: {rows:?}"
        );
        assert!(rows[2].contains("Second"), "{rows:?}");
    }

    /// A branch reaches past its siblings to the node it belongs to, so which one that is
    /// can be followed by eye.
    #[test]
    fn a_branch_reaches_back_to_its_parent() {
        let rows = drawn("mindmap\n  Root\n    One\n      Deep\n    Two");
        // `Two` belongs to `Root`, so its line runs past the rows `One` and `Deep` occupy.
        let last = rows.last().expect("a row for Two");
        assert!(last.contains("Two"), "{rows:?}");
        assert!(rows[2].contains('│'), "the branch passes through: {rows:?}");
    }

    /// Whatever brackets say about a node's shape, a terminal draws the words.
    #[test]
    fn a_shape_is_read_for_its_words() {
        assert_eq!(read_label("root((A cloud))").as_deref(), Some("A cloud"));
        assert_eq!(read_label("id[Square]").as_deref(), Some("Square"));
        assert_eq!(read_label("id(Round)").as_deref(), Some("Round"));
        assert_eq!(read_label("Just words").as_deref(), Some("Just words"));
    }

    /// What is not a mind map is refused rather than guessed at.
    #[test]
    fn what_is_not_a_mindmap_is_left_alone() {
        assert!(render_mindmap("graph TD\n  A --> B").is_none());
        assert!(
            render_mindmap("mindmap").is_none(),
            "nothing hangs off nothing"
        );
        assert!(
            render_mindmap("mindmap\n  Alone").is_none(),
            "one node is not a map"
        );
    }

    /// A map too large to take in is not drawn.
    #[test]
    fn too_many_nodes_are_refused() {
        let mut source = String::from("mindmap\n  Root\n");
        for index in 0..MAX_NODES {
            source.push_str(&format!("    Node {index}\n"));
        }
        assert!(render_mindmap(&source).is_none());
    }
}
