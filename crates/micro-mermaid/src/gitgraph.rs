//! Git graphs, drawn as commit marks on horizontal branch lanes.

use std::collections::HashMap;

use crate::canvas::{draw_text, Canvas, CONT, STY_DOT, STY_SOLID};
use crate::labels::{ascii_lower, ascii_upper, clean_label, fit_label, strip_controls};
use crate::types::Cls;
use crate::width::{measured, string_width};

/// Branches past this and the graph is refused: a history with hundreds of live branches has
/// nothing left to read as a shape.
const MAX_LANES: usize = 64;
/// Commits, merges and cherry-picks combined, across every lane.
const MAX_COMMITS: usize = 256;
/// Minimum columns between two commits on the same lane.
const GAP_MIN: usize = 4;
/// A tag or id longer than this is truncated, so one long label cannot blow out the column spacing
/// for the whole graph.
const LABEL_WIDTH: usize = 16;
const MAX_CANVAS_CELLS: usize = 1 << 21;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MarkKind {
    Normal,
    Reverse,
    Highlight,
    Merge,
    CherryPick,
}

struct Mark {
    lane: usize,
    col: usize,
    kind: MarkKind,
    id: Option<String>,
    tag: Option<String>,
}

enum Connector {
    /// A branch forking off its parent at `col`.
    Branch {
        child: usize,
        parent: usize,
        col: usize,
    },
    /// `from` merging into `to`, landing on the mark at `col`.
    Merge { from: usize, to: usize, col: usize },
    /// A cherry-pick copying the commit at `(from_lane, from_col)` onto the mark at `(to_lane,
    /// to_col)`.
    CherryPick {
        from_lane: usize,
        from_col: usize,
        to_lane: usize,
        to_col: usize,
    },
}

struct Lane {
    name: String,
    start_col: usize,
    last_col: usize,
    touched: bool,
}

/// The graph as it is built up statement by statement.
struct GitGraph {
    title: Option<String>,
    lanes: Vec<Lane>,
    lane_index: HashMap<String, usize>,
    marks: Vec<Mark>,
    connectors: Vec<Connector>,
    /// Commit ids that were given explicitly, for `cherry-pick` to find.
    id_index: HashMap<String, (usize, usize)>,
    current: usize,
    col: usize,
}

impl GitGraph {
    fn new() -> Self {
        let mut lane_index = HashMap::new();
        lane_index.insert("main".to_string(), 0);
        GitGraph {
            title: None,
            lanes: vec![Lane {
                name: "main".to_string(),
                start_col: 0,
                last_col: 0,
                touched: false,
            }],
            lane_index,
            marks: Vec::new(),
            connectors: Vec::new(),
            id_index: HashMap::new(),
            current: 0,
            col: 0,
        }
    }

    fn commit(&mut self, id: Option<String>, tag: Option<String>, kind: MarkKind) -> Option<()> {
        if self.marks.len() >= MAX_COMMITS {
            return None;
        }
        self.col += 1;
        let col = self.col;
        let lane = self.current;
        self.lanes[lane].last_col = col;
        self.lanes[lane].touched = true;
        if let Some(id) = &id {
            if self.id_index.contains_key(id) {
                return None;
            }
            self.id_index.insert(id.clone(), (lane, col));
        }
        self.marks.push(Mark {
            lane,
            col,
            kind,
            id,
            tag,
        });
        Some(())
    }

    fn branch(&mut self, name: &str) -> Option<()> {
        let name = name.trim();
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return None;
        }
        if self.lane_index.contains_key(name) || self.lanes.len() >= MAX_LANES {
            return None;
        }
        let parent = self.current;
        let start_col = self.lanes[parent].last_col;
        let child = self.lanes.len();
        self.lanes.push(Lane {
            name: name.to_string(),
            start_col,
            last_col: start_col,
            touched: false,
        });
        self.lane_index.insert(name.to_string(), child);
        self.connectors.push(Connector::Branch {
            child,
            parent,
            col: start_col,
        });

        self.current = child;
        Some(())
    }

    fn checkout(&mut self, name: &str) -> Option<()> {
        self.current = *self.lane_index.get(name.trim())?;
        Some(())
    }

    fn merge(&mut self, name: &str, id: Option<String>, tag: Option<String>) -> Option<()> {
        let from = *self.lane_index.get(name.trim())?;
        let to = self.current;
        if from == to || self.marks.len() >= MAX_COMMITS {
            return None;
        }
        self.col += 1;
        let col = self.col;
        self.lanes[from].last_col = self.lanes[from].last_col.max(col);
        self.lanes[from].touched = true;
        self.lanes[to].last_col = col;
        self.lanes[to].touched = true;
        if let Some(id) = &id {
            if self.id_index.contains_key(id) {
                return None;
            }
            self.id_index.insert(id.clone(), (to, col));
        }
        self.marks.push(Mark {
            lane: to,
            col,
            kind: MarkKind::Merge,
            id,
            tag,
        });
        self.connectors.push(Connector::Merge { from, to, col });
        Some(())
    }

    fn cherry_pick(&mut self, id: &str) -> Option<()> {
        let &(from_lane, from_col) = self.id_index.get(id)?;
        if self.marks.len() >= MAX_COMMITS {
            return None;
        }
        self.col += 1;
        let col = self.col;
        let to = self.current;
        self.lanes[to].last_col = col;
        self.lanes[to].touched = true;
        self.marks.push(Mark {
            lane: to,
            col,
            kind: MarkKind::CherryPick,
            id: None,
            tag: None,
        });
        self.connectors.push(Connector::CherryPick {
            from_lane,
            from_col,
            to_lane: to,
            to_col: col,
        });
        Some(())
    }
}

/// Draw `src` as a git graph, or answer nothing when it is not one.
pub(crate) fn render_gitgraph(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let mut lines = src.lines().map(str::trim).filter(|l| !l.is_empty());

    let header = lines.next()?;
    let word = header.split_whitespace().next()?;
    if ascii_lower(word.trim_end_matches(':')) != "gitgraph" {
        return None;
    }

    let mut graph = GitGraph::new();
    for line in lines {
        apply(line, &mut graph)?;
    }
    if graph.marks.is_empty() {
        return None;
    }
    draw(&graph)
}

fn take_field(body: &mut String, key: &str) -> Option<String> {
    let pat = format!("{key}:");
    let start = body.find(&pat)?;
    let after_key = start + pat.len();
    let after = &body[after_key..];
    let skip = after.len() - after.trim_start().len();
    let value_start = after_key + skip;
    let rest = &body[value_start..];
    let (value, value_end) = if let Some(quoted) = rest.strip_prefix('"') {
        let close = quoted.find('"')?;
        (quoted[..close].to_string(), value_start + close + 2)
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        (rest[..end].to_string(), value_start + end)
    };
    body.replace_range(start..value_end, "");
    Some(value)
}

fn commit_kind(token: &str) -> Option<MarkKind> {
    match ascii_upper(token).as_str() {
        "NORMAL" => Some(MarkKind::Normal),
        "REVERSE" => Some(MarkKind::Reverse),
        "HIGHLIGHT" => Some(MarkKind::Highlight),
        _ => None,
    }
}

/// Apply one statement line to `graph`.
fn apply(line: &str, graph: &mut GitGraph) -> Option<()> {
    let word = line.split_whitespace().next()?;
    let first = ascii_lower(word);
    let rest = line[word.len()..].trim();

    match first.as_str() {
        "title" => {
            graph.title = Some(clean_label(rest));
            Some(())
        }
        "commit" => {
            let mut body = rest.to_string();
            let id = take_field(&mut body, "id").map(|s| clean_label(&s));
            let tag = take_field(&mut body, "tag").map(|s| clean_label(&s));
            let kind = match take_field(&mut body, "type") {
                None => MarkKind::Normal,
                Some(token) => commit_kind(&token)?,
            };
            graph.commit(id, tag, kind)
        }
        "branch" => graph.branch(rest),
        "checkout" | "switch" => graph.checkout(rest),
        "merge" => {
            let mut body = rest.to_string();
            let id = take_field(&mut body, "id").map(|s| clean_label(&s));
            let tag = take_field(&mut body, "tag").map(|s| clean_label(&s));
            graph.merge(body.trim(), id, tag)
        }
        "cherry-pick" => {
            let mut body = rest.to_string();
            let id = take_field(&mut body, "id")?;
            graph.cherry_pick(&id)
        }
        _ => None,
    }
}

fn place(canvas: &mut Canvas, text: &str, row: usize, start_x: usize) {
    if row >= canvas.h {
        return;
    }
    let mut x = start_x;
    for (c, cw) in measured(text) {
        if cw == 0 || x + cw > canvas.w {
            break;
        }
        let blocked = (0..cw).any(|k| canvas.ch[canvas.idx(x + k, row)] != " ");
        if blocked {
            break;
        }
        canvas.set(x, row, c, Cls::EdgeLabel);
        for k in 1..cw {
            canvas.set(x + k, row, CONT, Cls::EdgeLabel);
        }
        x += cw;
    }
}

fn mark_label(mark: &Mark) -> Option<String> {
    let tag = mark
        .tag
        .as_deref()
        .map(|t| format!("({})", fit_label(t, LABEL_WIDTH)));
    let id = mark.id.as_deref().map(|i| fit_label(i, LABEL_WIDTH));
    match (tag, id) {
        (Some(tag), Some(id)) => Some(format!("{tag} {id}")),
        (Some(tag), None) => Some(tag),
        (None, Some(id)) => Some(id),
        (None, None) => None,
    }
}

fn glyph(kind: MarkKind) -> &'static str {
    match kind {
        MarkKind::Normal | MarkKind::Merge => "●",
        MarkKind::Highlight => "◆",
        MarkKind::Reverse => "×",
        MarkKind::CherryPick => "○",
    }
}

fn draw(graph: &GitGraph) -> Option<Canvas> {
    let label_w = graph
        .lanes
        .iter()
        .map(|l| string_width(&l.name))
        .max()
        .unwrap_or(0);
    let left = label_w + 1;

    let content_w = graph
        .marks
        .iter()
        .filter_map(mark_label)
        .map(|s| string_width(&s))
        .max()
        .unwrap_or(0);
    let gap = (content_w + 2).max(GAP_MIN);
    let x = |col: usize| left + col * gap;

    let title_h = usize::from(graph.title.is_some());
    let row = |lane: usize| title_h + lane;

    let mut canvas_w = (x(graph.col) + content_w + 2).max(left + 1);
    if let Some(title) = &graph.title {
        canvas_w = canvas_w.max(string_width(title));
    }
    let canvas_h = title_h + graph.lanes.len();
    if canvas_w.saturating_mul(canvas_h) > MAX_CANVAS_CELLS {
        return None;
    }

    let mut canvas = Canvas::new(canvas_w, canvas_h);
    if let Some(title) = &graph.title {
        draw_text(&mut canvas, title, 0, 0, Cls::Title);
    }

    for (i, lane) in graph.lanes.iter().enumerate() {
        draw_text(&mut canvas, &lane.name, 0, row(i), Cls::Text);
        let end = if lane.touched {
            lane.last_col
        } else {
            lane.start_col + 1
        };
        canvas.seg_h(row(i), x(lane.start_col), x(end));
    }

    for conn in &graph.connectors {
        match *conn {
            Connector::Branch { child, parent, col } => {
                canvas.seg_v(x(col), row(parent), row(child));
            }
            Connector::Merge { from, to, col } => {
                canvas.seg_v(x(col), row(from), row(to));
            }
            Connector::CherryPick {
                from_lane,
                from_col,
                to_lane,
                to_col,
            } => {
                canvas.cur_style = STY_DOT;
                canvas.seg_h(row(from_lane), x(from_col), x(to_col));
                canvas.seg_v(x(to_col), row(from_lane), row(to_lane));
                canvas.cur_style = STY_SOLID;
            }
        }
    }

    for mark in &graph.marks {
        canvas.set(x(mark.col), row(mark.lane), glyph(mark.kind), Cls::Edge);
    }
    for mark in &graph.marks {
        if let Some(label) = mark_label(mark) {
            place(&mut canvas, &label, row(mark.lane), x(mark.col) + 2);
        }
    }

    canvas.finalize_mask();
    Some(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_gitgraph(src)
            .expect("it is a git graph")
            .to_lines()
            .plain
    }

    /// Commits on the one branch mermaid starts with land in a single row, in order.
    #[test]
    fn a_gitgraph_draws_commits_along_the_main_lane() {
        let rows = drawn("gitGraph\n  commit\n  commit\n  commit");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].starts_with("main "), "{rows:?}");
        assert_eq!(rows[0].matches('●').count(), 3);
    }

    #[test]
    fn a_branch_forks_off_its_parent_lane() {
        let rows = drawn("gitGraph\n  commit\n  branch feature\n  commit");
        assert_eq!(rows.len(), 2);
        assert!(rows[0].starts_with("main "), "{rows:?}");
        assert!(rows[1].starts_with("feature "), "{rows:?}");

        let fork_col = rows[0].chars().position(|c| c == '●').unwrap();
        assert!(
            rows[1].chars().nth(fork_col).is_some_and(|c| c != ' '),
            "expected a connector under the fork point: {rows:?}"
        );
    }

    /// `merge` draws a joining line from the merged branch's lane into a new commit on the current
    /// one.
    #[test]
    fn a_merge_joins_two_branch_lanes() {
        let rows = drawn(
            "gitGraph\n  commit\n  branch feature\n  commit\n  checkout main\n  merge feature",
        );
        assert_eq!(rows.len(), 2);

        assert_eq!(rows[0].matches('●').count(), 2);
        assert_eq!(rows[1].matches('●').count(), 1);
        let merge_col = rows[0]
            .chars()
            .enumerate()
            .filter(|&(_, c)| c == '●')
            .last()
            .unwrap()
            .0;
        assert!(
            rows[1].chars().nth(merge_col).is_some_and(|c| c != ' '),
            "expected the feature lane to carry a connector under the merge point: {rows:?}"
        );
    }

    #[test]
    fn a_cherry_pick_draws_a_dotted_line_back_to_its_source() {
        let rows = drawn(
            "gitGraph\n  commit\n  branch feature\n  commit id: \"fix\"\n  checkout main\n  cherry-pick id: \"fix\"",
        );
        let joined = rows.join("\n");
        assert!(joined.contains('○'), "cherry-picked commit: {rows:?}");
        assert!(
            joined.contains('╌') || joined.contains('╎'),
            "expected a dotted connector: {rows:?}"
        );
    }

    /// `type: HIGHLIGHT` and `type: REVERSE` get their own glyphs so a reader can tell them apart
    /// from an ordinary commit at a glance.
    #[test]
    fn commit_types_get_distinct_glyphs() {
        let rows = drawn("gitGraph\n  commit type: HIGHLIGHT\n  commit type: REVERSE\n  commit");
        assert!(rows[0].contains('◆'), "{rows:?}");
        assert!(rows[0].contains('×'), "{rows:?}");
        assert!(rows[0].contains('●'), "{rows:?}");
    }

    /// A tagged commit shows its tag in parentheses beside the mark.
    #[test]
    fn a_tagged_commit_shows_its_tag_beside_the_mark() {
        let rows = drawn("gitGraph\n  commit tag: \"v1.0\"");
        assert!(rows[0].contains("(v1.0)"), "{rows:?}");
    }

    /// `title` labels the whole graph on its own row above the lanes.
    #[test]
    fn a_title_is_drawn_above_the_lanes() {
        let rows = drawn("gitGraph\n  title Release Train\n  commit");
        assert_eq!(rows[0], "Release Train");
        assert!(rows[1].starts_with("main "), "{rows:?}");
    }

    #[test]
    fn what_is_not_a_gitgraph_is_left_alone() {
        assert!(render_gitgraph("graph TD\n A --> B").is_none());
        assert!(render_gitgraph("gitGraph").is_none(), "no commits at all");
        assert!(render_gitgraph("gitGraph\n  commit\n  checkout nope").is_none());
        assert!(render_gitgraph("gitGraph\n  commit\n  merge nope").is_none());
        assert!(
            render_gitgraph("gitGraph\n  commit\n  cherry-pick id: \"nope\"").is_none(),
            "cherry-picking an id that was never committed"
        );
        assert!(render_gitgraph("gitGraph\n  commit type: SIDEWAYS").is_none());
        assert!(
            render_gitgraph("gitGraph\n  branch feature\n  branch feature").is_none(),
            "re-declaring a branch name"
        );
    }

    /// A history with more branches than can be read as a shape is refused.
    #[test]
    fn too_many_lanes_are_refused() {
        let mut src = String::from("gitGraph\n  commit\n");
        for i in 0..MAX_LANES {
            src.push_str(&format!("  branch f{i}\n"));
        }
        assert!(render_gitgraph(&src).is_none());
    }
}
