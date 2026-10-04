mod architecture;
mod block;
mod canvas;
mod gantt;
mod gitgraph;
mod graph;
mod journey;
mod kanban;
mod labels;
mod layout;
mod layout_seq;
mod mindmap;
mod packet;
mod parse;
mod pie;
mod quadrant;
mod radar;
mod requirement;
mod sankey;
mod source_box;
mod timeline;
mod treemap;
mod types;
mod width;
mod xychart;

pub use parse::{diagram_kind, DiagramKind};
pub use source_box::source_box;
pub use types::{Art, Cls, Span};

use graph::{ClassInfo, Dir, Graph};
use layout::{layout_class, layout_flowchart, layout_grouped, CanvasResult};
use layout_seq::layout_sequence;
use parse::{parse_class, parse_er, parse_graph, parse_sequence, parse_state};

/// Render a Mermaid source block as Unicode box-drawing art.
pub fn render(src: &str) -> Option<Art> {
    let src = labels::strip_controls(src);
    if src.trim().is_empty() {
        return None;
    }
    let drawn = attempt(&src)?;
    Some(art(drawn))
}

/// Render as `render` does, but when the written orientation is wider than `max_width`, try the
/// diagram turned the other way and keep that when it fits. `None` when nothing can be drawn.
pub fn render_fit(src: &str, max_width: usize) -> Option<Art> {
    let src = labels::strip_controls(src);
    if src.trim().is_empty() {
        return None;
    }
    let drawn = attempt(&src)?;
    if fits(&drawn, max_width) {
        return Some(art(drawn));
    }
    match attempt_turn(&src, turned) {
        Some(turned_drawn) if fits(&turned_drawn, max_width) => Some(art(turned_drawn)),
        _ => Some(art(drawn)),
    }
}

/// The lines of a drawn canvas as finished art.
fn art(drawn: Drawn) -> Art {
    let lines = drawn.canvas.to_lines();
    Art {
        plain: lines.plain,
        styled: lines.styled,
        width: lines.width,
        warnings: drawn.warnings,
    }
}

/// Whether a drawn canvas is no wider than `max_width`.
fn fits(drawn: &Drawn, max_width: usize) -> bool {
    drawn.canvas.w <= max_width
}

struct Drawn {
    canvas: canvas::Canvas,
    warnings: Vec<String>,
}

/// Draw `src` as written, retrying once without its last line if the grammar rejects it.
fn attempt(src: &str) -> Option<Drawn> {
    attempt_turn(src, |dir| dir)
}

/// Draw `src` with every orientation replaced by `turn`, retrying once without its last line if
/// the grammar rejects it.
fn attempt_turn(src: &str, turn: impl Copy + Fn(Dir) -> Dir) -> Option<Drawn> {
    if let Some(drawn) = draw_turn(src, &turn) {
        return Some(drawn);
    }

    let body = src.trim_end();
    let cut = body.rfind('\n')?;
    let salvaged = draw_turn(&body[..cut], &turn)?;

    let dropped = body[cut + 1..].trim();
    let mut warnings = salvaged.warnings;
    warnings.push(format!("dropped, unreadable final line: \"{dropped}\""));
    Some(Drawn {
        canvas: salvaged.canvas,
        warnings,
    })
}

/// Replace an orientation with the one across from it.
fn turned(dir: Dir) -> Dir {
    match dir {
        Dir::Down => Dir::Right,
        Dir::Up => Dir::Left,
        Dir::Right => Dir::Down,
        Dir::Left => Dir::Up,
    }
}

/// A flowchart or state diagram: plain boxes, no extra content.
fn drawn_flowchart(graph: &Graph) -> Option<Drawn> {
    let canvas = if graph.groups.is_empty() {
        layout_flowchart(graph)
    } else {
        layout_grouped(graph)
    };
    canvas.map(|canvas| Drawn {
        canvas,
        warnings: graph.warnings.clone(),
    })
}

/// A class or ER diagram: boxes divided into title / attribute / method rows.
fn drawn_class(graph: &Graph, infos: &[ClassInfo]) -> Option<Drawn> {
    layout_class(graph, infos).map(|canvas| Drawn {
        canvas,
        warnings: Vec::new(),
    })
}

/// Dispatch on the declared diagram type, applying `turn` to each orientation it names; `None`
/// means nothing was drawn.
fn draw_turn(src: &str, turn: &impl Fn(Dir) -> Dir) -> Option<Drawn> {
    fn plain(canvas: CanvasResult) -> Option<Drawn> {
        canvas.map(|canvas| Drawn {
            canvas,
            warnings: Vec::new(),
        })
    }

    match parse::diagram_kind(src)? {
        DiagramKind::Flowchart => {
            let mut graph = parse_graph(src)?;
            graph.dir = turn(graph.dir);
            drawn_flowchart(&graph)
        }

        DiagramKind::Pie => plain(pie::render_pie(src)),

        DiagramKind::Mindmap => plain(mindmap::render_mindmap(src)),
        DiagramKind::Timeline => plain(timeline::render_timeline(src)),
        DiagramKind::Journey => plain(journey::render_journey(src)),
        DiagramKind::Architecture => plain(architecture::render_architecture(src)),
        DiagramKind::Block => plain(block::render_block(src)),
        DiagramKind::GitGraph => plain(gitgraph::render_gitgraph(src)),
        DiagramKind::Kanban => plain(kanban::render_kanban(src)),
        DiagramKind::Packet => plain(packet::render_packet(src)),
        DiagramKind::Radar => plain(radar::render_radar(src)),
        DiagramKind::Sankey => plain(sankey::render_sankey(src)),
        DiagramKind::Treemap => plain(treemap::render_treemap(src)),
        DiagramKind::XyChart => plain(xychart::render_xychart(src)),
        DiagramKind::Gantt => plain(gantt::render_gantt(src)),
        DiagramKind::Quadrant => plain(quadrant::render_quadrant(src)),
        DiagramKind::Requirement => plain(requirement::render_requirement(src)),
        DiagramKind::State => {
            let mut state = parse_state(src)?;
            state.dir = turn(state.dir);
            drawn_flowchart(&state)
        }
        DiagramKind::Class => {
            let (mut graph, infos) = parse_class(src)?;
            graph.dir = turn(graph.dir);
            drawn_class(&graph, &infos)
        }
        DiagramKind::Er => {
            let (mut graph, infos) = parse_er(src)?;
            graph.dir = turn(graph.dir);
            drawn_class(&graph, &infos)
        }
        DiagramKind::Sequence => {
            let seq = parse_sequence(src)?;
            plain(layout_sequence(&seq))
        }
    }
}
