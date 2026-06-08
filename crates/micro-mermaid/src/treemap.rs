use crate::canvas::{draw_text, Canvas};
use crate::labels::{clean_label, strip_controls};
use crate::types::Cls;
use crate::width::string_width;

/// Nodes past this and the tree is refused, the same reasoning as `graph::MAX_NODES`.
const MAX_NODES: usize = 128;
/// Levels of nesting past this and the tree is refused: a treemap this deep is a filesystem, not a
/// diagram.
const MAX_DEPTH: usize = 32;

struct Node {
    label: String,
    explicit_value: Option<f64>,
    children: Vec<usize>,
}

struct Row {
    prefix: String,
    label: String,
    value_text: String,
}

/// Draw `src` as a treemap, or answer nothing when it is not one.
pub(crate) fn render_treemap(src: &str) -> Option<Canvas> {
    let (title, arena, roots) = parse_treemap(src)?;
    Some(draw(title.as_deref(), &arena, &roots))
}

fn parse_treemap(src: &str) -> Option<(Option<String>, Vec<Node>, Vec<usize>)> {
    let src = strip_controls(src);
    let mut lines = src.lines();
    if lines.next()?.trim() != "treemap-beta" {
        return None;
    }

    let mut title = None;
    let mut arena: Vec<Node> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();

    let mut stack: Vec<(usize, usize)> = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("title ") {
            title = Some(clean_label(rest.trim()));
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        while stack.last().is_some_and(|&(i, _)| i >= indent) {
            stack.pop();
        }
        if stack.len() >= MAX_DEPTH || arena.len() >= MAX_NODES {
            return None;
        }

        let (label, value) = read_entry(trimmed)?;
        let idx = arena.len();
        arena.push(Node {
            label,
            explicit_value: value,
            children: Vec::new(),
        });
        match stack.last() {
            Some(&(_, parent)) => arena[parent].children.push(idx),
            None => roots.push(idx),
        }
        stack.push((indent, idx));
    }

    if roots.is_empty() {
        return None;
    }
    Some((title, arena, roots))
}

/// `"Label"` on its own, or `"Label": value` when it carries one.
fn read_entry(line: &str) -> Option<(String, Option<f64>)> {
    if let Some((label_part, value_part)) = line.rsplit_once(':') {
        if let Ok(value) = value_part.trim().parse::<f64>() {
            let label = clean_label(label_part.trim());
            if !label.is_empty() && value.is_finite() {
                return Some((label, Some(value)));
            }
        }
    }
    let label = clean_label(line);
    if label.is_empty() {
        None
    } else {
        Some((label, None))
    }
}

/// A node's value: what it declared itself, or the sum of its children when it did not.
fn resolved(idx: usize, arena: &[Node], cache: &mut [Option<f64>]) -> f64 {
    if let Some(value) = cache[idx] {
        return value;
    }
    let node = &arena[idx];
    let value = if node.children.is_empty() {
        node.explicit_value.unwrap_or(0.0)
    } else {
        let sum: f64 = node
            .children
            .iter()
            .map(|&child| resolved(child, arena, cache))
            .sum();
        node.explicit_value.unwrap_or(sum)
    };
    cache[idx] = Some(value);
    value
}

fn draw(title: Option<&str>, arena: &[Node], roots: &[usize]) -> Canvas {
    let mut cache = vec![None; arena.len()];
    let values: Vec<f64> = (0..arena.len())
        .map(|i| resolved(i, arena, &mut cache))
        .collect();
    let grand_total: f64 = roots.iter().map(|&r| values[r]).sum();

    let mut rows = Vec::new();
    collect_rows(arena, roots, "", &values, grand_total, &mut rows);

    let top = usize::from(title.is_some());
    let width = rows
        .iter()
        .map(|r| string_width(&r.prefix) + string_width(&r.label) + 1 + string_width(&r.value_text))
        .max()
        .unwrap_or(0)
        .max(string_width(title.unwrap_or("")))
        .max(1);

    let mut canvas = Canvas::new(width, (top + rows.len()).max(1));
    if let Some(title) = title {
        draw_text(&mut canvas, title, 0, 0, Cls::Title);
    }
    for (i, row) in rows.iter().enumerate() {
        let y = top + i;
        draw_text(&mut canvas, &row.prefix, 0, y, Cls::Edge);
        let label_x = string_width(&row.prefix);
        draw_text(&mut canvas, &row.label, label_x, y, Cls::Text);
        let value_x = label_x + string_width(&row.label) + 1;
        draw_text(&mut canvas, &row.value_text, value_x, y, Cls::EdgeLabel);
    }
    canvas
}

fn collect_rows(
    arena: &[Node],
    indices: &[usize],
    prefix: &str,
    values: &[f64],
    parent_value: f64,
    rows: &mut Vec<Row>,
) {
    for (i, &idx) in indices.iter().enumerate() {
        let is_last = i + 1 == indices.len();
        let value = values[idx];
        let pct = if parent_value > 0.0 {
            value / parent_value * 100.0
        } else {
            0.0
        };
        rows.push(Row {
            prefix: format!("{prefix}{}", if is_last { "└─ " } else { "├─ " }),
            label: arena[idx].label.clone(),
            value_text: format!("({}, {pct:.0}%)", trim_number(value)),
        });
        let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
        collect_rows(
            arena,
            &arena[idx].children,
            &child_prefix,
            values,
            value,
            rows,
        );
    }
}

/// A value written the way it was meant: whole numbers without a decimal point.
fn trim_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_treemap(src)
            .expect("it is a treemap")
            .to_lines()
            .plain
    }

    /// A leaf's value and its share of its parent are written beside it.
    #[test]
    fn a_leaf_carries_its_value_and_share_of_its_parent() {
        let rows =
            drawn("treemap-beta\ntitle Budget\n\"Category A\"\n  \"Item 1\": 10\n  \"Item 2\": 30");
        assert_eq!(rows[0], "Budget");
        assert!(
            rows.iter()
                .any(|r| r.contains("Item 1") && r.contains("(10, 25%)")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|r| r.contains("Item 2") && r.contains("(30, 75%)")),
            "{rows:?}"
        );
    }

    /// A parent with no value of its own is worth the sum of its children.
    #[test]
    fn a_parent_with_no_value_sums_its_children() {
        let rows = drawn("treemap-beta\n\"Category A\"\n  \"Item 1\": 10\n  \"Item 2\": 30");
        assert!(
            rows.iter()
                .any(|r| r.contains("Category A") && r.contains("(40, 100%)")),
            "{rows:?}"
        );
    }

    #[test]
    fn tree_connectors_mark_the_last_branch() {
        let rows = drawn("treemap-beta\n\"A\": 1\n\"B\": 2");
        assert!(rows[0].starts_with("├─ "), "{rows:?}");
        assert!(rows[1].starts_with("└─ "), "{rows:?}");
    }

    /// A grandchild's percentage is of its own parent, not the grand total.
    #[test]
    fn a_grandchilds_share_is_of_its_own_parent() {
        let rows = drawn("treemap-beta\n\"A\"\n  \"B\"\n    \"C\": 5\n    \"D\": 5\n\"E\": 10");

        assert!(
            rows.iter()
                .any(|r| r.contains('C') && r.contains("(5, 50%)")),
            "{rows:?}"
        );
    }

    #[test]
    fn what_is_not_a_treemap_is_left_alone() {
        assert!(render_treemap("graph TD\n A --> B").is_none());
        assert!(render_treemap("treemap-beta").is_none(), "nothing in it");
        assert!(
            render_treemap("treemap-beta\n\"\"").is_none(),
            "an empty label"
        );
    }

    /// A tree this deep is a filesystem, not a diagram, so it is refused.
    #[test]
    fn too_deep_a_tree_is_refused() {
        let mut source = String::from("treemap-beta\n");
        for depth in 0..MAX_DEPTH + 1 {
            source.push_str(&"  ".repeat(depth));
            source.push_str(&format!("\"Level {depth}\": 1\n"));
        }
        assert!(render_treemap(&source).is_none());
    }
}
