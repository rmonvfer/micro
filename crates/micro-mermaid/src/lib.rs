

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
    let lines = drawn.canvas.to_lines();
    Some(Art {
        plain: lines.plain,
        styled: lines.styled,
        width: lines.width,
        warnings: drawn.warnings,
    })
}

struct Drawn {
    canvas: canvas::Canvas,
    warnings: Vec<String>,
}

/// Draw `src`, retrying once without its last line if the grammar rejects it.
fn attempt(src: &str) -> Option<Drawn> {
    if let Some(drawn) = draw(src) {
        return Some(drawn);
    }

    let body = src.trim_end();
    let cut = body.rfind('\n')?;
    let salvaged = draw(&body[..cut])?;

    let dropped = body[cut + 1..].trim();
    let mut warnings = salvaged.warnings;
    warnings.push(format!("dropped, unreadable final line: \"{dropped}\""));
    Some(Drawn {
        canvas: salvaged.canvas,
        warnings,
    })
}

/// Dispatch on the declared diagram type; `None` means nothing was drawn.
fn draw(src: &str) -> Option<Drawn> {
    fn plain(canvas: CanvasResult) -> Option<Drawn> {
        canvas.map(|canvas| Drawn {
            canvas,
            warnings: Vec::new(),
        })
    }

    match parse::diagram_kind(src)? {
        DiagramKind::Flowchart => {
            let graph = parse_graph(src)?;
            let canvas = if graph.groups.is_empty() {
                layout_flowchart(&graph)
            } else {
                layout_grouped(&graph)
            };
            canvas.map(|canvas| Drawn {
                canvas,
                warnings: graph.warnings,
            })
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
            let state = parse_state(src)?;
            plain(layout_flowchart(&state))
        }
        DiagramKind::Class => {
            let (graph, infos) = parse_class(src)?;
            plain(layout_class(&graph, &infos))
        }
        DiagramKind::Er => {
            let (graph, infos) = parse_er(src)?;
            plain(layout_class(&graph, &infos))
        }
        DiagramKind::Sequence => {
            let seq = parse_sequence(src)?;
            plain(layout_sequence(&seq))
        }
    }
}
