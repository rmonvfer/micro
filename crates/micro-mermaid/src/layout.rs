//! Graph layout: rank, order, place, route, draw.
//!
//! Follows the Sugiyama outline — assign ranks along the flow axis, reorder
//! within ranks to cut crossings, then relax positions on the cross axis so
//! chains stay straight. Edges between adjacent ranks share horizontal "bus"
//! rows; everything else is routed around the diagram through vertical
//! "lanes".
//!
//! `BT` and `RL` reuse the `TD`/`LR` layouts and flip the finished canvas, so
//! text never ends up mirrored.

use std::collections::HashMap;

use crate::canvas::{
    draw_text, draw_text_over_edges, Canvas, CONT, D, L, R, STY_DOT, STY_SOLID, STY_THICK, U,
};
use crate::graph::{ClassInfo, Dir, Edge, Graph, Head, LineKind, Node, Shape};
use crate::labels::{display_generics, fit_label, wrap_label, MAX_LABEL, MAX_LINES, WRAP_WIDTH};
use crate::types::Cls;
use crate::width::{measured, string_width};

/// Cells of padding between a box border and its text.
const PAD: usize = 1;
/// Minimum horizontal / vertical space between boxes.
const GAP_X: usize = 3;
const GAP_Y: usize = 2;
/// Refuse to allocate a canvas larger than this many cells.
const MAX_CANVAS_CELLS: usize = 1 << 21;

/// Saturating subtraction, so a layout invariant that turns out wrong reads as
/// a shifted diagram rather than a panic.
fn sat(a: usize, b: usize) -> usize {
    a.saturating_sub(b)
}

fn half(n: usize) -> usize {
    n / 2
}

/// A laid-out canvas, or `None` when the diagram is empty or over the cell cap.
pub type CanvasResult = Option<Canvas>;

#[derive(Debug, Clone, Copy, Default)]
pub struct Placed {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub cx: usize,
    pub cy: usize,
    pub rank: usize,
}

/// Per-node dimensions. `lay_*` include room for self-edge loops and labels.
struct NodeSizes {
    box_w: Vec<usize>,
    box_h: Vec<usize>,
    lay_w: Vec<usize>,
    lay_h: Vec<usize>,
    extra_h: Vec<usize>,
    self_label_w: Vec<usize>,
}

/// What to draw inside a node box.
pub enum NodeExtra {
    Plain,
    Frame { sub: Canvas },
    Compartments { sections: Vec<Vec<String>> },
}

struct RoutePlan {
    canvas_w: usize,
    canvas_h: usize,
    /// Coordinate just past each rank's boxes, where its bus rows begin.
    band_end: Vec<usize>,
    /// Bus track offset per edge.
    edge_bus: Vec<usize>,
    /// Coordinate of the first lane track.
    lane_base: usize,
    /// Lane track offset per edge.
    edge_lane: Vec<usize>,
}

// ------------------------------------------------------------------ ranking

/// Longest-path ranking over the graph's DAG.
///
/// Back edges (those closing a cycle) are excluded by a DFS colouring pass, so
/// `A --> B --> C --> A` still ranks 0, 1, 2 rather than diverging.
pub fn compute_ranks(graph: &Graph) -> Vec<usize> {
    let n = graph.nodes.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for e in &graph.edges {
        if e.from != e.to {
            children[e.from].push(e.to);
            indeg[e.to] += 1;
        }
    }

    let mut color = vec![0u8; n];
    let mut dag: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut order: Vec<usize> = Vec::new();

    // Roots first so ranks grow from natural entry points, then any leftovers.
    let roots: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    for start in roots.into_iter().chain(0..n) {
        if color[start] == 0 {
            dfs_dag(start, &children, &mut color, &mut dag, &mut order);
        }
    }

    let mut rank = vec![0usize; n];
    for &u in order.iter().rev() {
        for &v in &dag[u] {
            rank[v] = rank[v].max(rank[u] + 1);
        }
    }
    rank
}

/// Iterative DFS recording postorder and skipping edges back into the stack.
fn dfs_dag(
    start: usize,
    children: &[Vec<usize>],
    color: &mut [u8],
    dag: &mut [Vec<usize>],
    order: &mut Vec<usize>,
) {
    let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
    color[start] = 1;
    while let Some(&(u, i)) = stack.last() {
        if i < children[u].len() {
            let v = children[u][i];
            stack.last_mut().unwrap().1 += 1;
            if color[v] == 1 {
                continue; // grey: a back edge, ignore it
            }
            dag[u].push(v);
            if color[v] == 0 {
                color[v] = 1;
                stack.push((v, 0));
            }
        } else {
            color[u] = 2;
            order.push(u);
            stack.pop();
        }
    }
}

/// Reorder nodes within each rank to minimise edge crossings (barycenter
/// sweeps): alternate down/up passes sort each rank by the mean position of
/// its neighbours, keeping whichever ordering crossed least.
pub fn order_ranks(by_rank: &mut [Vec<usize>], edges: &[Edge], ranks: &[usize]) {
    let n = ranks.len();
    if by_rank.len() < 2 || n < 3 {
        return;
    }

    let mut parents: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in edges {
        if e.from != e.to && ranks[e.to] > ranks[e.from] {
            parents[e.to].push(e.from);
            children[e.from].push(e.to);
        }
    }

    let mut pos = vec![0usize; n];
    let reindex = |row: &[usize], pos: &mut [usize]| {
        for (i, &v) in row.iter().enumerate() {
            pos[v] = i;
        }
    };
    for row in by_rank.iter() {
        reindex(row, &mut pos);
    }

    let mut best: Vec<Vec<usize>> = by_rank.to_vec();
    let mut best_crossings = count_crossings(edges, ranks, &pos);
    if best_crossings == 0 {
        return;
    }

    for it in 0..8 {
        // Alternate sweeping down (sort by parents) and up (sort by children).
        let row_indices: Vec<usize> = if it % 2 == 0 {
            (1..by_rank.len()).collect()
        } else {
            (0..by_rank.len() - 1).rev().collect()
        };
        let neigh = if it % 2 == 0 { &parents } else { &children };
        for &ri in &row_indices {
            sort_by_barycenter(&mut by_rank[ri], neigh, &pos);
            reindex(&by_rank[ri], &mut pos);
        }
        let crossings = count_crossings(edges, ranks, &pos);
        if crossings < best_crossings {
            best_crossings = crossings;
            best = by_rank.to_vec();
        }
        if best_crossings == 0 {
            break;
        }
    }

    for (i, row) in by_rank.iter_mut().enumerate() {
        *row = std::mem::take(&mut best[i]);
    }
}

fn sort_by_barycenter(row: &mut [usize], neigh: &[Vec<usize>], pos: &[usize]) {
    let mut keyed: Vec<(f64, usize)> = row
        .iter()
        .map(|&v| {
            let key = if neigh[v].is_empty() {
                pos[v] as f64
            } else {
                neigh[v].iter().map(|&u| pos[u] as f64).sum::<f64>() / neigh[v].len() as f64
            };
            (key, v)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    for (i, &(_, v)) in keyed.iter().enumerate() {
        row[i] = v;
    }
}

pub fn count_crossings(edges: &[Edge], ranks: &[usize], pos: &[usize]) -> usize {
    let adjacent: Vec<(usize, usize, usize)> = edges
        .iter()
        .filter(|e| e.from != e.to && ranks[e.to] == ranks[e.from] + 1)
        .map(|e| (ranks[e.from], pos[e.from], pos[e.to]))
        .collect();
    let mut crossings = 0usize;
    for i in 0..adjacent.len() {
        let a = adjacent[i];
        for b in &adjacent[i + 1..] {
            if a.0 == b.0 && ((a.1 < b.1 && a.2 > b.2) || (a.1 > b.1 && a.2 < b.2)) {
                crossings += 1;
            }
        }
    }
    crossings
}

/// Assign a cross-axis centre to every node so nodes line up under their
/// neighbours: each node drifts toward the average of its neighbours while
/// ranks keep their order and boxes keep `sep` between them.
fn assign_positions(
    by_rank: &[Vec<usize>],
    size: &[usize],
    sep: usize,
    edges: &[Edge],
    ranks: &[usize],
) -> Vec<usize> {
    let n = size.len();
    let mut parents: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in edges {
        if e.from != e.to && ranks[e.to] > ranks[e.from] {
            parents[e.to].push(e.from);
            children[e.from].push(e.to);
        }
    }

    let mut pos = vec![0.0f64; n];
    for row in by_rank {
        let mut x = 0.0f64;
        for &v in row {
            let h = size[v] as f64 / 2.0;
            x += h;
            pos[v] = x;
            x += h + sep as f64;
        }
    }

    for it in 0..10 {
        let neigh = if it % 2 == 0 { &parents } else { &children };
        if it % 2 == 0 {
            for row in by_rank {
                relax_rank(row, neigh, &mut pos, size, sep);
            }
        } else {
            for row in by_rank.iter().rev() {
                relax_rank(row, neigh, &mut pos, size, sep);
            }
        }
    }

    let mut min_left = f64::INFINITY;
    for v in 0..n {
        min_left = min_left.min(pos[v] - size[v] as f64 / 2.0);
    }
    if !min_left.is_finite() {
        min_left = 0.0;
    }
    (0..n).map(|v| (pos[v] - min_left).round().max(0.0) as usize).collect()
}

fn relax_rank(nodes: &[usize], neigh: &[Vec<usize>], pos: &mut [f64], size: &[usize], sep: usize) {
    let n = nodes.len();
    if n == 0 {
        return;
    }

    let desired: Vec<f64> = nodes
        .iter()
        .map(|&v| {
            if neigh[v].is_empty() {
                pos[v]
            } else {
                neigh[v].iter().map(|&u| pos[u]).sum::<f64>() / neigh[v].len() as f64
            }
        })
        .collect();
    let half_of = |i: usize| size[nodes[i]] as f64 / 2.0;

    // Sweep right then left, then take the midpoint: this centres a node between
    // the tightest packing that respects order from either side.
    let mut left = vec![0.0f64; n];
    for i in 0..n {
        left[i] = if i == 0 {
            desired[i]
        } else {
            desired[i].max(left[i - 1] + half_of(i - 1) + sep as f64 + half_of(i))
        };
    }
    let mut right = vec![0.0f64; n];
    for i in (0..n).rev() {
        right[i] = if i == n - 1 {
            desired[i]
        } else {
            desired[i].min(right[i + 1] - half_of(i + 1) - sep as f64 - half_of(i))
        };
    }
    for i in 0..n {
        pos[nodes[i]] = (left[i] + right[i]) / 2.0;
    }
    for i in 1..n {
        let min_p = pos[nodes[i - 1]] + half_of(i - 1) + sep as f64 + half_of(i);
        if pos[nodes[i]] < min_p {
            pos[nodes[i]] = min_p;
        }
    }
}

// ------------------------------------------------------------------- tracks

/// A span competing for a track: `(start, end, from, to, edge_index)`.
type Span5 = (usize, usize, usize, usize, usize);

struct TrackAssignment {
    assigned: Vec<(usize, usize)>,
    count: usize,
}

/// Pack spans into as few parallel tracks as possible.
///
/// Two spans share a track when they are two cells apart, or when they share
/// an endpoint — edges fanning out of one node deliberately reuse a single row
/// so a merge draws one arrowhead rather than a stack of them.
fn assign_tracks(spans: &[Span5]) -> TrackAssignment {
    let mut sorted: Vec<Span5> = spans.to_vec();
    sorted.sort();

    let mut tracks: Vec<Vec<(usize, usize, usize, usize)>> = Vec::new();
    let mut assigned: Vec<(usize, usize)> = Vec::new();
    for &(s, e, f, t, idx) in &sorted {
        let slot = tracks.iter().position(|members| {
            members.iter().all(|&(s2, e2, f2, t2)| e2 + 2 <= s || e + 2 <= s2 || f2 == f || t2 == t)
        });
        let slot = match slot {
            Some(slot) => slot,
            None => {
                tracks.push(Vec::new());
                tracks.len() - 1
            }
        };
        tracks[slot].push((s, e, f, t));
        assigned.push((idx, slot));
    }
    TrackAssignment { assigned, count: tracks.len() }
}

/// Edges from rank `r` to `r + 1` that must jog sideways, so need a bus row.
fn bus_spans(graph: &Graph, ranks: &[usize], centers: &[usize], r: usize, exact: bool) -> Vec<Span5> {
    let mut out = Vec::new();
    for (i, e) in graph.edges.iter().enumerate() {
        let jogs = if exact {
            centers[e.from] != centers[e.to]
        } else {
            centers[e.from].abs_diff(centers[e.to]) > 1
        };
        if e.from != e.to && ranks[e.from] == r && ranks[e.to] == r + 1 && jogs {
            out.push((
                centers[e.from].min(centers[e.to]),
                centers[e.from].max(centers[e.to]),
                e.from,
                e.to,
                i,
            ));
        }
    }
    out
}

/// Edges skipping a rank or running backwards; these go around in a lane.
fn lane_spans(graph: &Graph, ranks: &[usize], placed: &[Placed], vertical: bool) -> Vec<Span5> {
    let mut out = Vec::new();
    for (i, e) in graph.edges.iter().enumerate() {
        if e.from == e.to || ranks[e.to] == ranks[e.from] + 1 {
            continue;
        }
        let pf = &placed[e.from];
        let pt = &placed[e.to];
        let (a, b) = if vertical {
            (pf.cy.min(pt.cy), pf.cy.max(pt.cy))
        } else {
            (pf.cx.min(pt.cx), pf.cx.max(pt.cx))
        };
        out.push((a, b, e.from, e.to, i));
    }
    out
}

// ----------------------------------------------------------------- placement

fn place_td(
    ranks: &[usize],
    max_rank: usize,
    by_rank: &[Vec<usize>],
    sizes: &NodeSizes,
    graph: &Graph,
    placed: &mut [Placed],
) -> RoutePlan {
    let centers = assign_positions(by_rank, &sizes.lay_w, GAP_X, &graph.edges, ranks);

    let mut edge_bus = vec![0usize; graph.edges.len()];
    let mut bus_tracks = vec![0usize; max_rank + 1];
    for (r, tracks) in bus_tracks.iter_mut().enumerate().take(max_rank) {
        let spans = bus_spans(graph, ranks, &centers, r, false);
        if spans.is_empty() {
            continue;
        }
        let track = assign_tracks(&spans);
        for &(idx, slot) in &track.assigned {
            edge_bus[idx] = slot;
        }
        *tracks = track.count;
    }

    let rank_h: Vec<usize> = by_rank
        .iter()
        .map(|row| {
            if row.is_empty() {
                3
            } else {
                row.iter().map(|&i| sizes.box_h[i] + sizes.extra_h[i]).max().unwrap()
            }
        })
        .collect();
    let mut rank_y = vec![0usize; max_rank + 1];
    for r in 1..=max_rank {
        rank_y[r] = rank_y[r - 1] + rank_h[r - 1] + GAP_Y.max(bus_tracks[r - 1] + 1);
    }
    let canvas_h = rank_y[max_rank] + rank_h[max_rank];
    let band_end: Vec<usize> = (0..=max_rank).map(|r| rank_y[r] + rank_h[r]).collect();

    let mut diagram_w = 1usize;
    for (r, row) in by_rank.iter().enumerate() {
        for &idx in row {
            let w = sizes.box_w[idx];
            let h = sizes.box_h[idx];
            let cx = centers[idx];
            let x = sat(cx, half(w));
            let y = rank_y[r] + half(rank_h[r] - h - sizes.extra_h[idx]);
            placed[idx] = Placed { x, y, w, h, cx, cy: y + half(h), rank: r };
            diagram_w = diagram_w.max(x + w);
            if sizes.extra_h[idx] > 0 && sizes.self_label_w[idx] > 0 {
                diagram_w = diagram_w.max(x + w + 2 + sizes.self_label_w[idx]);
            }
        }
    }

    let mut content_w = diagram_w;
    for e in &graph.edges {
        if e.from == e.to {
            continue;
        }
        let Some(label) = &e.label else { continue };
        let lw = string_width(label).min(MAX_LABEL);
        content_w = if ranks[e.to] == ranks[e.from] + 1 {
            content_w.max(placed[e.to].cx + 2 + lw)
        } else {
            content_w.max(diagram_w + lw + 1)
        };
    }

    let mut edge_lane = vec![0usize; graph.edges.len()];
    let lanes = lane_spans(graph, ranks, placed, true);
    let mut canvas_w = content_w;
    let mut lane_base = 0usize;
    if !lanes.is_empty() {
        let track = assign_tracks(&lanes);
        for &(idx, slot) in &track.assigned {
            edge_lane[idx] = slot;
        }
        canvas_w = content_w + 1 + track.count;
        lane_base = content_w + 1;
    }

    RoutePlan { canvas_w, canvas_h, band_end, edge_bus, lane_base, edge_lane }
}

fn place_lr(
    ranks: &[usize],
    max_rank: usize,
    by_rank: &[Vec<usize>],
    sizes: &NodeSizes,
    graph: &Graph,
    placed: &mut [Placed],
) -> RoutePlan {
    let col_w: Vec<usize> = by_rank
        .iter()
        .map(|row| {
            if row.is_empty() {
                0
            } else {
                row.iter().map(|&i| sizes.box_w[i]).max().unwrap()
            }
        })
        .collect();

    // Left-to-right edge labels sit in the gap between columns, so the gap has
    // to be wide enough for the widest of them.
    let label_widths: Vec<usize> = graph
        .edges
        .iter()
        .filter(|e| e.from == e.to || ranks[e.to] == ranks[e.from] + 1)
        .filter_map(|e| e.label.as_deref())
        .map(|label| string_width(label).min(MAX_LABEL))
        .collect();
    let max_label = label_widths.iter().copied().max().unwrap_or(0);
    let base_gap = (GAP_X + 1).max(max_label + 3);

    let centers = assign_positions(by_rank, &sizes.lay_h, 1, &graph.edges, ranks);

    let mut edge_bus = vec![0usize; graph.edges.len()];
    let mut bus_tracks = vec![0usize; max_rank + 1];
    for (r, tracks) in bus_tracks.iter_mut().enumerate().take(max_rank) {
        let spans = bus_spans(graph, ranks, &centers, r, true);
        if spans.is_empty() {
            continue;
        }
        let track = assign_tracks(&spans);
        for &(idx, slot) in &track.assigned {
            edge_bus[idx] = slot;
        }
        *tracks = track.count;
    }

    let mut rank_x = vec![0usize; max_rank + 1];
    for r in 1..=max_rank {
        rank_x[r] = rank_x[r - 1] + col_w[r - 1] + base_gap.max(bus_tracks[r - 1] + 1);
    }
    let self_tails: Vec<usize> = by_rank[max_rank]
        .iter()
        .filter(|&&i| sizes.extra_h[i] > 0 && sizes.self_label_w[i] > 0)
        .map(|&i| 2 + sizes.self_label_w[i])
        .collect();
    let canvas_w =
        rank_x[max_rank] + col_w[max_rank] + self_tails.iter().copied().max().unwrap_or(0);
    let band_end: Vec<usize> = (0..=max_rank).map(|r| rank_x[r] + col_w[r]).collect();

    let mut diagram_h = 1usize;
    for (r, row) in by_rank.iter().enumerate() {
        let x = rank_x[r];
        for &idx in row {
            let w = sizes.box_w[idx];
            let h = sizes.box_h[idx];
            let cy = centers[idx];
            let y = sat(cy, half(h + sizes.extra_h[idx]));
            placed[idx] = Placed { x, y, w, h, cx: x + half(w), cy: y + half(h), rank: r };
            diagram_h = diagram_h.max(y + h + sizes.extra_h[idx]);
        }
    }

    let mut edge_lane = vec![0usize; graph.edges.len()];
    let lanes = lane_spans(graph, ranks, placed, false);
    let mut canvas_h = diagram_h;
    let mut lane_base = 0usize;
    if !lanes.is_empty() {
        let track = assign_tracks(&lanes);
        for &(idx, slot) in &track.assigned {
            edge_lane[idx] = slot;
        }
        canvas_h = diagram_h + 1 + track.count;
        lane_base = diagram_h + 1;
    }

    RoutePlan { canvas_w, canvas_h, band_end, edge_bus, lane_base, edge_lane }
}

// -------------------------------------------------------------------- canvas

/// Rank, place, draw and route a graph onto a fresh canvas.
pub fn layout_canvas(graph: &Graph, extras: &[NodeExtra]) -> CanvasResult {
    let n = graph.nodes.len();
    if n == 0 {
        return None;
    }

    let ranks = compute_ranks(graph);
    let max_rank = ranks.iter().copied().max().unwrap_or(0);

    let mut by_rank: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (idx, &r) in ranks.iter().enumerate() {
        by_rank[r].push(idx);
    }
    order_ranks(&mut by_rank, &graph.edges, &ranks);

    let wrapped: Vec<Vec<String>> =
        graph.nodes.iter().map(|node| wrap_label(&node.label, WRAP_WIDTH, MAX_LINES)).collect();
    let widest = |lines: &[String]| -> usize {
        if lines.is_empty() {
            1
        } else {
            lines.iter().map(|l| string_width(l)).max().unwrap_or(0).max(1)
        }
    };

    let mut box_w: Vec<usize> = extras
        .iter()
        .enumerate()
        .map(|(i, extra)| match extra {
            NodeExtra::Frame { sub } => {
                (sub.w + 2).max(string_width(&fit_label(&graph.nodes[i].label, WRAP_WIDTH)) + 4)
            }
            NodeExtra::Compartments { sections } => {
                let flat: Vec<String> = sections.iter().flatten().cloned().collect();
                widest(&flat) + 2 * PAD + 2
            }
            NodeExtra::Plain => widest(&wrapped[i]) + 2 * PAD + 2,
        })
        .collect();
    let box_h: Vec<usize> = extras
        .iter()
        .enumerate()
        .map(|(i, extra)| match extra {
            NodeExtra::Frame { sub } => sub.h + 2,
            NodeExtra::Compartments { sections } => {
                let filled = sections.iter().filter(|s| !s.is_empty()).count();
                sections.iter().map(|s| s.len()).sum::<usize>() + sat(filled, 1) + 2
            }
            NodeExtra::Plain => wrapped[i].len() + 2,
        })
        .collect();

    // A self-edge needs two rows below its box, and room beside it for a label.
    let mut extra_h = vec![0usize; n];
    let mut self_label_w = vec![0usize; n];
    for e in &graph.edges {
        if e.from != e.to {
            continue;
        }
        extra_h[e.from] = 2;
        if let Some(label) = &e.label {
            self_label_w[e.from] = self_label_w[e.from].max(string_width(label).min(MAX_LABEL));
        }
    }
    for i in 0..n {
        if extra_h[i] > 0 {
            box_w[i] = box_w[i].max(7);
        }
    }

    let sizes = NodeSizes {
        lay_w: box_w
            .iter()
            .enumerate()
            .map(|(i, &w)| w + if self_label_w[i] > 0 { 2 * (self_label_w[i] + 3) } else { 0 })
            .collect(),
        lay_h: box_h.iter().enumerate().map(|(i, &h)| h + extra_h[i]).collect(),
        box_w,
        box_h,
        extra_h,
        self_label_w,
    };

    let mut placed: Vec<Placed> = vec![Placed::default(); n];

    let vertical = matches!(graph.dir, Dir::Down | Dir::Up);
    let plan = if vertical {
        place_td(&ranks, max_rank, &by_rank, &sizes, graph, &mut placed)
    } else {
        place_lr(&ranks, max_rank, &by_rank, &sizes, graph, &mut placed)
    };

    if plan.canvas_w * plan.canvas_h > MAX_CANVAS_CELLS {
        return None;
    }

    let mut canvas = Canvas::new(plan.canvas_w, plan.canvas_h);
    for idx in 0..n {
        match &extras[idx] {
            NodeExtra::Frame { sub } => {
                draw_frame(&mut canvas, &placed[idx], &graph.nodes[idx].label, sub)
            }
            NodeExtra::Compartments { sections } => {
                draw_class_box(&mut canvas, &placed[idx], sections)
            }
            NodeExtra::Plain => {
                draw_box(&mut canvas, &placed[idx], &wrapped[idx], graph.nodes[idx].shape)
            }
        }
    }

    for (i, edge) in graph.edges.iter().enumerate() {
        canvas.cur_style = match edge.line {
            LineKind::Dotted => STY_DOT,
            LineKind::Thick => STY_THICK,
            LineKind::Solid => STY_SOLID,
        };
        if edge.from == edge.to {
            route_self(&mut canvas, &placed[edge.from], edge);
            continue;
        }
        let from = placed[edge.from];
        let to = placed[edge.to];
        let adjacent = to.rank == from.rank + 1;
        let bus = plan.band_end[from.rank] + plan.edge_bus[i];
        let lane = plan.lane_base + plan.edge_lane[i];
        if vertical {
            if adjacent {
                route_forward(&mut canvas, &from, &to, edge, bus);
            } else {
                route_back(&mut canvas, &from, &to, edge, lane);
            }
        } else if adjacent {
            route_forward_lr(&mut canvas, &from, &to, edge, bus);
        } else {
            route_back_lr(&mut canvas, &from, &to, edge, lane);
        }
    }

    canvas.finalize_mask();
    Some(canvas)
}

/// Apply the direction flip a finished canvas needs for `BT` / `RL`.
pub fn orient(mut canvas: Canvas, graph: &Graph) -> Canvas {
    match graph.dir {
        Dir::Up => canvas.flip_vertical(),
        Dir::Left => canvas.flip_horizontal(),
        _ => {}
    }
    canvas
}

/// Flowchart and state diagrams: plain boxes, no extra content.
pub fn layout_flowchart(graph: &Graph) -> CanvasResult {
    let extras: Vec<NodeExtra> = graph.nodes.iter().map(|_| NodeExtra::Plain).collect();
    layout_canvas(graph, &extras).map(|canvas| orient(canvas, graph))
}

/// Class and ER diagrams: boxes divided into title / attribute / method rows.
pub fn layout_class(graph: &Graph, infos: &[ClassInfo]) -> CanvasResult {
    let extras: Vec<NodeExtra> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let mut title = Vec::new();
            if let Some(annotation) = &infos[i].annotation {
                title.push(format!("«{annotation}»"));
            }
            title.push(display_generics(&node.label));
            NodeExtra::Compartments {
                sections: vec![title, infos[i].attrs.clone(), infos[i].methods.clone()],
            }
        })
        .collect();
    layout_canvas(graph, &extras).map(|canvas| orient(canvas, graph))
}

// -------------------------------------------------------------------- groups

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ItemKey {
    Node(usize),
    Group(usize),
}

struct Endpoint {
    key: ItemKey,
    chain: Vec<usize>,
}

/// Lay out a flowchart that uses `subgraph`.
///
/// Each subgraph becomes a framed box holding its own independently laid-out
/// canvas. An edge is drawn in the innermost scope containing both endpoints;
/// one crossing a subgraph boundary attaches to the frame instead of the node.
pub fn layout_grouped(graph: &Graph) -> CanvasResult {
    // A node whose id matches a subgraph id stands in for that subgraph.
    let mut proxy: HashMap<usize, usize> = HashMap::new();
    for (gi, g) in graph.groups.iter().enumerate() {
        if let Some(&ni) = graph.index.get(&g.id) {
            proxy.insert(ni, gi);
        }
    }

    let group_chain = |mut g: Option<usize>| -> Vec<usize> {
        let mut chain = Vec::new();
        while let Some(cur) = g {
            chain.push(cur);
            g = graph.groups[cur].parent;
        }
        chain.reverse();
        chain
    };
    let endpoint = |n: usize| -> Endpoint {
        match proxy.get(&n) {
            None => Endpoint { key: ItemKey::Node(n), chain: group_chain(graph.node_group[n]) },
            Some(&gi) => {
                Endpoint { key: ItemKey::Group(gi), chain: group_chain(graph.groups[gi].parent) }
            }
        }
    };

    // Edges bucketed by the scope that draws them; `None` is the top level.
    let mut scope_edges: HashMap<Option<usize>, Vec<(ItemKey, ItemKey, usize)>> = HashMap::new();
    let mut referenced = vec![false; graph.groups.len()];
    for (ei, e) in graph.edges.iter().enumerate() {
        let f = endpoint(e.from);
        let t = endpoint(e.to);
        let mut k = 0;
        while k < f.chain.len() && k < t.chain.len() && f.chain[k] == t.chain[k] {
            k += 1;
        }
        let scope = if k == 0 { None } else { Some(f.chain[k - 1]) };
        let f_key = if f.chain.len() > k { ItemKey::Group(f.chain[k]) } else { f.key };
        let t_key = if t.chain.len() > k { ItemKey::Group(t.chain[k]) } else { t.key };
        for key in [f_key, t_key] {
            if let ItemKey::Group(gi) = key {
                referenced[gi] = true;
            }
        }
        scope_edges.entry(scope).or_default().push((f_key, t_key, ei));
    }

    let mut direct_nodes: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
    for (ni, &g) in graph.node_group.iter().enumerate() {
        if proxy.contains_key(&ni) {
            continue;
        }
        direct_nodes.entry(g).or_default().push(ni);
    }

    // Drop empty subgraphs, but keep any that an edge attaches to.
    let mut keep = vec![false; graph.groups.len()];
    for gi in (0..graph.groups.len()).rev() {
        let has_nodes = direct_nodes.get(&Some(gi)).is_some_and(|v| !v.is_empty());
        let has_children =
            graph.groups.iter().enumerate().any(|(c, g)| g.parent == Some(gi) && keep[c]);
        keep[gi] = has_nodes || has_children || referenced[gi];
    }

    let canvas = build_scope(graph, None, &scope_edges, &direct_nodes, &keep)?;
    Some(orient(canvas, graph))
}

fn build_scope(
    graph: &Graph,
    scope: Option<usize>,
    scope_edges: &HashMap<Option<usize>, Vec<(ItemKey, ItemKey, usize)>>,
    direct_nodes: &HashMap<Option<usize>, Vec<usize>>,
    keep: &[bool],
) -> CanvasResult {
    let mut items: Vec<ItemKey> =
        direct_nodes.get(&scope).into_iter().flatten().map(|&ni| ItemKey::Node(ni)).collect();
    let child_groups: Vec<usize> = (0..graph.groups.len())
        .filter(|&gi| graph.groups[gi].parent == scope && keep[gi])
        .collect();
    items.extend(child_groups.into_iter().map(ItemKey::Group));

    if items.is_empty() {
        return Some(Canvas::new(1, 1));
    }

    let mut index_of: HashMap<ItemKey, usize> = HashMap::new();
    let mut nodes: Vec<Node> = Vec::new();
    let mut extras: Vec<NodeExtra> = Vec::new();
    for &item in &items {
        index_of.insert(item, nodes.len());
        match item {
            ItemKey::Node(i) => {
                nodes.push(Node {
                    label: graph.nodes[i].label.clone(),
                    shape: graph.nodes[i].shape,
                });
                extras.push(NodeExtra::Plain);
            }
            ItemKey::Group(i) => {
                let sub = build_scope(graph, Some(i), scope_edges, direct_nodes, keep)?;
                nodes.push(Node { label: graph.groups[i].label.clone(), shape: Shape::Rect });
                extras.push(NodeExtra::Frame { sub });
            }
        }
    }

    let mut edges: Vec<Edge> = Vec::new();
    if let Some(list) = scope_edges.get(&scope) {
        for &(f, t, ei) in list {
            let (Some(&fi), Some(&ti)) = (index_of.get(&f), index_of.get(&t)) else { continue };
            let e = &graph.edges[ei];
            edges.push(Edge {
                from: fi,
                to: ti,
                label: e.label.clone(),
                head_to: e.head_to,
                head_from: e.head_from,
                line: e.line,
            });
        }
    }

    // Layout only reads nodes/edges/dir, so a bare Graph carrying those is enough.
    let mut synth = Graph::new(graph.dir);
    synth.nodes = nodes;
    synth.edges = edges;
    layout_canvas(&synth, &extras)
}

// ------------------------------------------------------------------- drawing

pub fn draw_box(canvas: &mut Canvas, p: &Placed, lines: &[String], shape: Shape) {
    let Placed { x, y, w, h, .. } = *p;
    let right = x + w - 1;
    let bottom = y + h - 1;

    let rounded = matches!(shape, Shape::Round | Shape::Diamond);
    canvas.set(x, y, if rounded { "╭" } else { "┌" }, Cls::Border);
    canvas.set(right, y, if rounded { "╮" } else { "┐" }, Cls::Border);
    canvas.set(x, bottom, if rounded { "╰" } else { "└" }, Cls::Border);
    canvas.set(right, bottom, if rounded { "╯" } else { "┘" }, Cls::Border);

    // The perimeter is drawn as bits so edges can tee into it, but it is the box
    // outline, so it claims `border` rather than `edge`.
    for cx in x + 1..right {
        canvas.add_bits(cx, y, L | R, Cls::Border);
        canvas.add_bits(cx, bottom, L | R, Cls::Border);
    }
    for cy in y + 1..bottom {
        canvas.add_bits(x, cy, U | D, Cls::Border);
        canvas.add_bits(right, cy, U | D, Cls::Border);
    }

    for cy in y..=bottom {
        for cx in x..=right {
            let i = canvas.idx(cx, cy);
            canvas.occupied[i] = true;
        }
    }

    let inner = sat(w, 2 * PAD + 2).max(1);
    for (li, line) in lines.iter().enumerate() {
        let text = fit_label(line, inner);
        let text_x = x + 1 + PAD + half(sat(inner, string_width(&text)));
        draw_text(canvas, &text, text_x, y + 1 + li, Cls::Text);
    }
}

/// A class or ER box: sections separated by horizontal rules, title centred.
fn draw_class_box(canvas: &mut Canvas, p: &Placed, sections: &[Vec<String>]) {
    draw_box(canvas, p, &[], Shape::Rect);
    let inner = sat(p.w, 2 * PAD + 2).max(1);
    let mut row = p.y + 1;
    let mut first = true;
    for (si, section) in sections.iter().enumerate() {
        if section.is_empty() {
            continue;
        }
        if !first {
            canvas.set(p.x, row, "├", Cls::Border);
            for x in p.x + 1..p.x + p.w - 1 {
                canvas.set(x, row, "─", Cls::Border);
            }
            canvas.set(p.x + p.w - 1, row, "┤", Cls::Border);
            row += 1;
        }
        first = false;
        for line in section {
            let text = fit_label(line, inner);
            let tx = if si == 0 {
                p.x + 1 + PAD + half(sat(inner, string_width(&text)))
            } else {
                p.x + 1 + PAD
            };
            draw_text_over_edges(canvas, &text, tx, row, Cls::Text);
            row += 1;
        }
    }
}

/// A subgraph frame: a titled box with a finished sub-canvas centred inside.
fn draw_frame(canvas: &mut Canvas, p: &Placed, title: &str, sub: &Canvas) {
    draw_box(canvas, p, &[], Shape::Rect);
    let t = fit_label(title, sat(p.w, 4));
    draw_text_over_edges(canvas, &format!(" {t} "), p.x + 1, p.y, Cls::Text);
    canvas.blit(sub, p.x + 1 + half(p.w - 2 - sub.w), p.y + 1 + half(p.h - 2 - sub.h));
}

// ------------------------------------------------------------------- routing

fn head_glyph(head: Head, arrow: &str) -> String {
    match head {
        Head::Circle => "o".to_string(),
        Head::Cross => "×".to_string(),
        Head::DiamondFill => "◆".to_string(),
        Head::DiamondOpen => "◇".to_string(),
        Head::Triangle => match arrow {
            "▼" => "▽",
            "▲" => "△",
            "◄" => "◁",
            "▶" => "▷",
            other => other,
        }
        .to_string(),
        _ => arrow.to_string(),
    }
}

/// Adjacent ranks, top-down: drop, jog along the bus row, drop into the head.
fn route_forward(canvas: &mut Canvas, from: &Placed, to: &Placed, edge: &Edge, bus: usize) {
    let tx = to.cx;
    // A jog of one column reads as a kink; snap straight instead.
    let bx = if from.cx.abs_diff(tx) <= 1 { tx } else { from.cx };
    let by = from.y + from.h - 1;
    let head_row = to.y - 1;

    canvas.junction(bx, by, D);
    canvas.seg_v(bx, by, bus);
    if bx == tx {
        canvas.seg_v(bx, bus, head_row);
    } else {
        canvas.seg_h(bus, bx, tx);
        canvas.seg_v(tx, bus, head_row);
    }

    if edge.head_to == Head::None {
        canvas.add_edge_bits(tx, head_row, U);
    } else {
        canvas.set(tx, head_row, &head_glyph(edge.head_to, "▼"), Cls::Edge);
    }
    if edge.head_from != Head::None {
        canvas.set(bx, by, &head_glyph(edge.head_from, "▲"), Cls::Edge);
    }

    if let Some(label) = &edge.label {
        place_label(canvas, label, head_row, tx + 1);
    }
}

/// A self-edge: a stub loop hanging below the box.
fn route_self(canvas: &mut Canvas, p: &Placed, edge: &Edge) {
    let bottom = p.y + p.h - 1;
    let exit_x = p.cx + 1;
    let ret_x = p.x + p.w - 2;
    if ret_x <= exit_x || bottom + 2 >= canvas.h {
        return;
    }

    let (v, h, bl, br) = match edge.line {
        LineKind::Dotted => ("╎", "╌", "╰", "╯"),
        LineKind::Thick => ("┃", "━", "┗", "┛"),
        LineKind::Solid => ("│", "─", "╰", "╯"),
    };

    canvas.junction(exit_x, bottom, D);
    canvas.set(exit_x, bottom + 1, v, Cls::Edge);
    canvas.set(exit_x, bottom + 2, bl, Cls::Edge);
    for x in exit_x + 1..ret_x {
        canvas.set(x, bottom + 2, h, Cls::Edge);
    }
    canvas.set(ret_x, bottom + 2, br, Cls::Edge);
    canvas.set(ret_x, bottom + 1, &head_glyph(edge.head_to, "▲"), Cls::Edge);
    if let Some(label) = &edge.label {
        place_label(canvas, label, bottom + 1, p.x + p.w + 1);
    }
}

/// Skip or back edge, top-down: out the right side, up a lane, back in.
fn route_back(canvas: &mut Canvas, from: &Placed, to: &Placed, edge: &Edge, lane_x: usize) {
    let sx = from.x + from.w - 1;
    let sy = from.cy;
    let tx = to.x + to.w - 1;
    let tyc = to.cy;

    canvas.junction(sx, sy, R);
    canvas.seg_h(sy, sx, lane_x);
    canvas.seg_v(lane_x, sy, tyc);
    canvas.seg_h(tyc, tx + 1, lane_x);

    if edge.head_to == Head::None {
        canvas.add_edge_bits(tx + 1, tyc, R);
    } else {
        canvas.set(tx + 1, tyc, &head_glyph(edge.head_to, "◄"), Cls::Edge);
    }
    if edge.head_from != Head::None {
        canvas.set(sx, sy, &head_glyph(edge.head_from, "◄"), Cls::Edge);
    }

    if let Some(label) = &edge.label {
        place_label(canvas, label, sat(tyc, 1), sat(lane_x, string_width(label) + 1));
    }
}

/// Adjacent ranks, left-to-right: out the right side, jog on the bus column.
fn route_forward_lr(canvas: &mut Canvas, from: &Placed, to: &Placed, edge: &Edge, bus: usize) {
    let rx = from.x + from.w - 1;
    let ry = from.cy;
    let ly = to.cy;
    let head_col = to.x - 1;

    canvas.junction(rx, ry, R);
    canvas.seg_h(ry, rx, bus);
    if ry == ly {
        canvas.seg_h(ry, bus, head_col);
    } else {
        canvas.seg_v(bus, ry, ly);
        canvas.seg_h(ly, bus, head_col);
    }

    if edge.head_to == Head::None {
        canvas.add_edge_bits(head_col, ly, R);
    } else {
        canvas.set(head_col, ly, &head_glyph(edge.head_to, "▶"), Cls::Edge);
    }
    if edge.head_from != Head::None {
        canvas.set(rx, ry, &head_glyph(edge.head_from, "◄"), Cls::Edge);
    }

    if let Some(label) = &edge.label {
        place_label(canvas, label, sat(ly, 1), bus + 1);
    }
}

/// Skip or back edge, left-to-right: down out the bottom, along a lane, back up.
fn route_back_lr(canvas: &mut Canvas, from: &Placed, to: &Placed, edge: &Edge, lane_y: usize) {
    let sx = from.cx;
    let sy = from.y + from.h - 1;
    let tx = to.cx;
    let ty = to.y + to.h - 1;

    canvas.junction(sx, sy, D);
    canvas.seg_v(sx, sy, lane_y);
    canvas.seg_h(lane_y, sx, tx);
    canvas.seg_v(tx, lane_y, ty + 1);

    if edge.head_to == Head::None {
        canvas.add_edge_bits(tx, ty + 1, D);
    } else {
        canvas.set(tx, ty + 1, &head_glyph(edge.head_to, "▲"), Cls::Edge);
    }
    if edge.head_from != Head::None {
        canvas.set(sx, sy, &head_glyph(edge.head_from, "▲"), Cls::Edge);
    }

    if let Some(label) = &edge.label {
        place_label(canvas, label, sat(lane_y, 1), half(sx + tx));
    }
}

/// Write an edge label, stopping at the first cell already occupied.
fn place_label(canvas: &mut Canvas, label: &str, row: usize, start_x: usize) {
    if row >= canvas.h {
        return;
    }
    let text = fit_label(label, MAX_LABEL);
    let mut x = start_x;
    for (c, cw) in measured(&text) {
        if cw == 0 {
            continue;
        }
        if x + cw > canvas.w {
            break;
        }
        let mut blocked = false;
        for k in 0..cw {
            let i = canvas.idx(x + k, row);
            if canvas.ch[i] != " " || canvas.mask[i] != 0 || canvas.occupied[i] {
                blocked = true;
            }
        }
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
