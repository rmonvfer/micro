//! Block diagrams, drawn as a grid of boxes with arrows between them.
//!
//! A block diagram already declares its own layout — `columns N` says how
//! many boxes make a row, so there is no ranking or crossing-reduction to do
//! the way a flowchart needs. What is left is the part flowchart layout
//! already solves well: a bordered box for one block (`draw_box`), a framed
//! box holding a grid of its own for a `block:id … end` group, and a
//! right-angle line between any two boxes once both have a position. Every
//! block, wherever it is nested, ends up with one absolute position on the
//! root canvas, so an arrow can join any two of them without having to know
//! or care which group either one is inside.

use std::collections::HashMap;

use crate::canvas::{draw_text_over_edges, Canvas, D, L, R, U};
use crate::graph::Shape;
use crate::labels::{ascii_lower, clean_label, fit_label, is_id_char, strip_controls, wrap_label, MAX_LINES, WRAP_WIDTH};
use crate::layout::{draw_box, Placed};
use crate::types::Cls;
use crate::width::string_width;

/// Blocks and groups combined, past which there is nothing left to read as a
/// grid — the spirit of `MAX_NODES` in `graph.rs`, applied to a different
/// kind of item.
const MAX_CELLS: usize = 128;
/// `block:id` nested inside `block:id` this many times and the diagram is
/// refused rather than laid out.
const MAX_DEPTH: usize = 6;
/// Space between grid cells, wide enough for an arrow's right-angle bend to
/// have a row or column of its own to turn in.
const GRID_GAP: usize = 3;
const PAD: usize = 1;
const MAX_CANVAS_CELLS: usize = 1 << 21;

struct Leaf {
    id: String,
    label: String,
    shape: Shape,
}

enum Cell {
    Leaf(Leaf),
    Space,
    Group {
        id: String,
        label: String,
        columns: Option<usize>,
        children: Vec<Cell>,
    },
}

enum Content {
    Leaf { lines: Vec<String>, shape: Shape },
    Space,
    Group { label: String, children: Vec<(usize, usize, MeasuredCell)> },
}

struct MeasuredCell {
    id: Option<String>,
    w: usize,
    h: usize,
    content: Content,
}

/// An open `block:id … end` scope, still collecting its children.
struct OpenGroup {
    id: String,
    label: String,
    columns: Option<usize>,
    children: Vec<Cell>,
}

struct Parser {
    top_columns: Option<usize>,
    top_children: Vec<Cell>,
    stack: Vec<OpenGroup>,
    arrows: Vec<(String, String)>,
    cell_count: usize,
}

impl Parser {
    fn new() -> Self {
        Parser {
            top_columns: None,
            top_children: Vec::new(),
            stack: Vec::new(),
            arrows: Vec::new(),
            cell_count: 0,
        }
    }

    fn current_children(&mut self) -> &mut Vec<Cell> {
        match self.stack.last_mut() {
            Some(group) => &mut group.children,
            None => &mut self.top_children,
        }
    }

    fn set_columns(&mut self, n: usize) {
        match self.stack.last_mut() {
            Some(group) => group.columns = Some(n),
            None => self.top_columns = Some(n),
        }
    }
}

/// Draw `src` as a block diagram, or answer nothing when it is not one.
pub(crate) fn render_block(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let mut lines = src.lines().map(str::trim).filter(|l| !l.is_empty());

    let header = lines.next()?;
    if ascii_lower(header.split_whitespace().next()?) != "block-beta" {
        return None;
    }

    let mut parser = Parser::new();
    for line in lines {
        apply(line, &mut parser)?;
    }
    // An unclosed `block:` is a broken diagram, not a partial one.
    if !parser.stack.is_empty() || parser.top_children.is_empty() {
        return None;
    }

    draw(&parser)
}

fn apply(line: &str, p: &mut Parser) -> Option<()> {
    let lower = ascii_lower(line);

    if lower.starts_with("block:") {
        if p.stack.len() >= MAX_DEPTH || p.cell_count >= MAX_CELLS {
            return None;
        }
        p.cell_count += 1;
        let (id, label, _) = parse_head(line["block:".len()..].trim());
        if id.is_empty() {
            return None;
        }
        p.stack.push(OpenGroup {
            id,
            label: label.unwrap_or_default(),
            columns: None,
            children: Vec::new(),
        });
        return Some(());
    }

    if lower == "end" {
        let group = p.stack.pop()?;
        let label = if group.label.is_empty() { group.id.clone() } else { group.label };
        p.current_children().push(Cell::Group {
            id: group.id,
            label,
            columns: group.columns,
            children: group.children,
        });
        return Some(());
    }

    let word = line.split_whitespace().next()?;
    let first = ascii_lower(word);

    if first == "columns" {
        let n: usize = line[word.len()..].trim().parse().ok()?;
        if n == 0 {
            return None;
        }
        p.set_columns(n);
        return Some(());
    }

    if first == "space" {
        if p.cell_count >= MAX_CELLS {
            return None;
        }
        p.cell_count += 1;
        p.current_children().push(Cell::Space);
        return Some(());
    }

    if let Some(arrow) = parse_arrow(line) {
        p.arrows.push(arrow);
        return Some(());
    }

    // A row of a block diagram is written as several blocks on one line — that is what
    // `columns` is counting — so the line is split before any of it is read as a block.
    for declaration in declarations(line) {
        if p.cell_count >= MAX_CELLS {
            return None;
        }
        if ascii_lower(declaration) == "space" {
            p.cell_count += 1;
            p.current_children().push(Cell::Space);
            continue;
        }
        let (id, label, shape) = parse_head(declaration);
        if id.is_empty() {
            return None;
        }
        p.cell_count += 1;
        let label = label.unwrap_or_else(|| id.clone());
        p.current_children().push(Cell::Leaf(Leaf { id, label, shape }));
    }
    Some(())
}

/// The block declarations on one line.
///
/// Blocks are separated by spaces, but a label may hold spaces of its own, so the split
/// only happens outside brackets: `A["One thing"] B["Another"]` is two blocks, not four.
fn declarations(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = None;

    for (at, character) in line.char_indices() {
        match character {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        match character.is_whitespace() && depth == 0 {
            true => {
                if let Some(from) = start.take() {
                    out.push(&line[from..at]);
                }
            }
            false => {
                start.get_or_insert(at);
            }
        }
    }
    if let Some(from) = start {
        out.push(&line[from..]);
    }
    out
}

/// `id`, `id[Label]`, `id(Label)`, `id((Label))` or `id{Label}` — enough of
/// flowchart's bracket vocabulary to shape a block, without needing that
/// grammar's full generality.
fn parse_head(s: &str) -> (String, Option<String>, Shape) {
    let id_end = s.find(|c: char| !is_id_char(c)).unwrap_or(s.len());
    let id = s[..id_end].to_string();
    let rest = s[id_end..].trim();
    if let Some(inner) = rest.strip_prefix("((").and_then(|r| r.strip_suffix("))")) {
        return (id, Some(clean_label(inner)), Shape::Round);
    }
    if let Some(inner) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return (id, Some(clean_label(inner)), Shape::Rect);
    }
    if let Some(inner) = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        return (id, Some(clean_label(inner)), Shape::Round);
    }
    if let Some(inner) = rest.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        return (id, Some(clean_label(inner)), Shape::Diamond);
    }
    (id, None, Shape::Rect)
}

/// `A --> B`, ignoring anything past the target id — an edge label on a
/// block arrow has nowhere natural to sit once the boxes are placed by grid
/// position rather than by rank, so it is read and set aside.
fn parse_arrow(line: &str) -> Option<(String, String)> {
    let (lhs, rhs) = line.split_once("-->")?;
    let from = lhs.trim();
    if from.is_empty() || !from.chars().all(is_id_char) {
        return None;
    }
    let rhs = rhs.trim();
    let to_end = rhs.find(|c: char| !is_id_char(c)).unwrap_or(rhs.len());
    let to = &rhs[..to_end];
    if to.is_empty() {
        return None;
    }
    Some((from.to_string(), to.to_string()))
}

fn measure(cell: &Cell) -> MeasuredCell {
    match cell {
        Cell::Space => MeasuredCell {
            id: None,
            w: 1,
            h: 1,
            content: Content::Space,
        },
        Cell::Leaf(leaf) => {
            let lines = wrap_label(&leaf.label, WRAP_WIDTH, MAX_LINES);
            let w = lines.iter().map(|l| string_width(l)).max().unwrap_or(0).max(1) + 2 * PAD + 2;
            let h = lines.len() + 2;
            MeasuredCell {
                id: Some(leaf.id.clone()),
                w,
                h,
                content: Content::Leaf { lines, shape: leaf.shape },
            }
        }
        Cell::Group {
            id,
            label,
            columns,
            children,
        } => {
            let measured: Vec<MeasuredCell> = children.iter().map(measure).collect();
            let cols = columns.unwrap_or(measured.len().max(1));
            let (placed, inner_w, inner_h) = pack_grid(measured, cols);
            let title_w = string_width(label) + 4;
            MeasuredCell {
                id: Some(id.clone()),
                w: inner_w.max(title_w) + 2,
                h: inner_h + 2,
                content: Content::Group {
                    label: label.clone(),
                    children: placed,
                },
            }
        }
    }
}

/// Lay `cells` into a grid of `columns` columns, filled row-major, every cell
/// sharing the width and height of the widest and tallest one: a block
/// diagram reads as a table, and a table's cells line up.
fn pack_grid(cells: Vec<MeasuredCell>, columns: usize) -> (Vec<(usize, usize, MeasuredCell)>, usize, usize) {
    let cell_w = cells.iter().map(|c| c.w).max().unwrap_or(1);
    let cell_h = cells.iter().map(|c| c.h).max().unwrap_or(1);
    let mut placed = Vec::with_capacity(cells.len());
    let mut inner_w = 0;
    let mut inner_h = 0;
    for (i, cell) in cells.into_iter().enumerate() {
        let (row, col) = (i / columns, i % columns);
        let x = col * (cell_w + GRID_GAP);
        let y = row * (cell_h + GRID_GAP);
        inner_w = inner_w.max(x + cell_w);
        inner_h = inner_h.max(y + cell_h);
        placed.push((x, y, cell));
    }
    (placed, inner_w, inner_h)
}

fn place(canvas: &mut Canvas, cell: &MeasuredCell, x: usize, y: usize, positions: &mut HashMap<String, (usize, usize, usize, usize)>) {
    if let Some(id) = &cell.id {
        positions.insert(id.clone(), (x, y, cell.w, cell.h));
    }
    let p = Placed {
        x,
        y,
        w: cell.w,
        h: cell.h,
        cx: x + cell.w / 2,
        cy: y + cell.h / 2,
        rank: 0,
    };
    match &cell.content {
        Content::Space => {}
        Content::Leaf { lines, shape } => draw_box(canvas, &p, lines, *shape),
        Content::Group { label, children } => {
            draw_box(canvas, &p, &[], Shape::Rect);
            let title = fit_label(label, cell.w.saturating_sub(4));
            draw_text_over_edges(canvas, &format!(" {title} "), x + 1, y, Cls::Text);
            for (cx, cy, child) in children {
                place(canvas, child, x + 1 + cx, y + 1 + cy, positions);
            }
        }
    }
}

fn draw(parser: &Parser) -> Option<Canvas> {
    let measured: Vec<MeasuredCell> = parser.top_children.iter().map(measure).collect();
    let columns = parser.top_columns.unwrap_or(measured.len().max(1));
    let (placed, w, h) = pack_grid(measured, columns);
    if w.saturating_mul(h) > MAX_CANVAS_CELLS {
        return None;
    }

    let mut canvas = Canvas::new(w, h);
    let mut positions = HashMap::new();
    for (x, y, cell) in &placed {
        place(&mut canvas, cell, *x, *y, &mut positions);
    }

    for (from, to) in &parser.arrows {
        let (&f, &t) = (positions.get(from)?, positions.get(to)?);
        connect(&mut canvas, f, t);
    }

    canvas.finalize_mask();
    Some(canvas)
}

/// Join two boxes with a single right-angle bend, entering whichever side of
/// the target faces the source: below/above when the vertical gap is the
/// larger one, left/right otherwise, matching how the grid itself reads —
/// row-major, so a jump to a later row is the common case.
///
/// The line routes through open space with `seg_v`/`seg_h`, which will not
/// draw over a box's own cells — but a block nested in a group sits flush
/// against that group's border, with no open cell of its own to land on. The
/// two ends that actually touch a box, source and target alike, go through
/// `junction` instead, the same way flowchart routing meets a box border:
/// that call adds its bit even to a cell a box has already claimed.
///
/// What this does not do is route around a third box sitting between source
/// and target: the bend is always the midpoint of the straight span between
/// them, so a line that has to pass near unrelated content on its way can
/// come out short where that content blocks it, the same way a flowchart's
/// own edges are not guaranteed clear of everything they pass near. The
/// arrowhead itself is drawn unconditionally, so the target is always
/// visibly the one being pointed to even on a route that loses a few cells.
fn connect(canvas: &mut Canvas, from: (usize, usize, usize, usize), to: (usize, usize, usize, usize)) {
    let (fx, fy, fw, fh) = from;
    let (tx, ty, tw, th) = to;
    let f_cx = fx + fw / 2;
    let f_cy = fy + fh / 2;
    let t_cx = tx + tw / 2;
    let t_cy = ty + th / 2;
    let dx = t_cx as isize - f_cx as isize;
    let dy = t_cy as isize - f_cy as isize;

    if dy.unsigned_abs() >= dx.unsigned_abs() && dy != 0 {
        let going_down = dy > 0;
        let sy = if going_down { fy + fh - 1 } else { fy };
        let ty2 = if going_down { ty } else { ty + th - 1 };
        let bend = sy.midpoint(ty2);
        canvas.junction(f_cx, sy, if going_down { D } else { U });
        canvas.seg_v(f_cx, sy, bend);
        canvas.seg_h(bend, f_cx.min(t_cx), f_cx.max(t_cx));
        canvas.seg_v(t_cx, bend, ty2);
        canvas.junction(t_cx, ty2, if going_down { D } else { U });
        canvas.set(t_cx, ty2, if going_down { "▼" } else { "▲" }, Cls::Edge);
    } else if dx != 0 {
        let going_right = dx > 0;
        let sx = if going_right { fx + fw - 1 } else { fx };
        let tx2 = if going_right { tx } else { tx + tw - 1 };
        let bend = sx.midpoint(tx2);
        canvas.junction(sx, f_cy, if going_right { R } else { L });
        canvas.seg_h(f_cy, sx, bend);
        canvas.seg_v(bend, f_cy.min(t_cy), f_cy.max(t_cy));
        canvas.seg_h(t_cy, bend, tx2);
        canvas.junction(tx2, t_cy, if going_right { R } else { L });
        canvas.set(tx2, t_cy, if going_right { "▶" } else { "◄" }, Cls::Edge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_block(src).expect("it is a block diagram").to_lines().plain
    }

    /// `columns N` arranges the blocks into a grid that many wide, wrapping
    /// to a new row after each `N` blocks.
    #[test]
    fn columns_wraps_blocks_into_a_grid() {
        let rows = drawn("block-beta\n  columns 2\n  a\n  b\n  c");
        let joined = rows.join("\n");
        assert!(joined.contains('a') && joined.contains('b') && joined.contains('c'));
        // Three blocks at two columns: the third starts a second row, so it
        // is drawn below the first rather than beside it.
        let row_of = |c: char| rows.iter().position(|r| r.contains(c)).unwrap();
        assert!(row_of('c') > row_of('a'), "{rows:?}");
    }

    /// `space` reserves a grid cell without drawing into it, so the block
    /// after it lands one slot further along instead of right beside its
    /// neighbour.
    #[test]
    fn space_leaves_a_gap_in_the_grid() {
        let with_space = drawn("block-beta\n  columns 3\n  a\n  space\n  b");
        let without = drawn("block-beta\n  columns 3\n  a\n  b");
        let col_of = |rows: &[String], c: char| rows.iter().find_map(|r| r.chars().position(|ch| ch == c)).unwrap();
        assert!(col_of(&with_space, 'b') > col_of(&without, 'b'));
    }

    /// A `block:id … end` group is drawn as a frame around its own grid of
    /// children, labelled with its id.
    #[test]
    fn a_block_group_frames_its_children() {
        let rows = drawn("block-beta\n  block:grp\n    a\n    b\n  end");
        let joined = rows.join("\n");
        assert!(joined.contains("grp"), "{rows:?}");
        assert!(joined.contains('a') && joined.contains('b'), "{rows:?}");
        assert!(joined.contains('┌') && joined.contains('┘'), "framed: {rows:?}");
    }

    /// An arrow between two top-level blocks is drawn as a right-angle line
    /// with an arrowhead at the target.
    #[test]
    fn an_arrow_joins_two_blocks() {
        let rows = drawn("block-beta\n  columns 1\n  a\n  b\n  a --> b");
        let joined = rows.join("\n");
        assert!(joined.contains('▼'), "{rows:?}");
    }

    /// An arrow can reach a block nested inside a group just as well as a
    /// top-level one, because every block gets one absolute position
    /// regardless of how deep it is declared.
    #[test]
    fn an_arrow_can_reach_into_a_group() {
        let rows = drawn("block-beta\n  columns 1\n  a\n  block:grp\n    b\n  end\n  a --> b");
        let joined = rows.join("\n");
        assert!(joined.contains('▼'), "{rows:?}");
    }

    /// A labelled block draws its label inside the box, same as a flowchart node.
    #[test]
    fn a_labelled_block_shows_its_label() {
        let rows = drawn("block-beta\n  a[\"Start Here\"]");
        assert!(rows.join("\n").contains("Start Here"), "{rows:?}");
    }

    /// Anything that is not a block diagram, or breaks down partway through,
    /// is refused rather than drawn wrong.
    #[test]
    fn what_is_not_a_block_diagram_is_left_alone() {
        assert!(render_block("graph TD\n A --> B").is_none());
        assert!(render_block("block-beta").is_none(), "no blocks at all");
        assert!(render_block("block-beta\n  block:grp\n    a").is_none(), "unclosed group");
        assert!(render_block("block-beta\n  end").is_none(), "end with nothing open");
        assert!(render_block("block-beta\n  a\n  a --> nope").is_none(), "arrow to an undeclared id");
        assert!(render_block("block-beta\n  columns 0\n  a").is_none());
    }

    /// A diagram with more blocks than can be read as a grid is refused.
    #[test]
    fn too_many_blocks_are_refused() {
        let mut src = String::from("block-beta\n  columns 8\n");
        for i in 0..MAX_CELLS + 1 {
            src.push_str(&format!("  n{i}\n"));
        }
        assert!(render_block(&src).is_none());
    }
}
