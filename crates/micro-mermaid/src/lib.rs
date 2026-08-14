//! Mermaid diagrams rendered as Unicode box-drawing art, for display in a
//! terminal or anywhere else colour comes from semantic spans rather than
//! embedded escape codes.
//!
//! [`render`] is the entry point: give it a fenced-off `mermaid` code block
//! and get back rows of text plus the same rows as classified spans. When it
//! declines — an unsupported diagram type, a syntax error, or a diagram too
//! large to lay out — [`source_box`] frames the raw source instead, which is
//! always a valid fallback view.

mod canvas;
mod gantt;
mod graph;
mod journey;
mod labels;
mod layout;
mod layout_seq;
mod mindmap;
mod parse;
mod pie;
mod quadrant;
mod requirement;
mod source_box;
mod timeline;
mod types;
mod width;

pub use parse::{diagram_kind, DiagramKind};
pub use source_box::source_box;
pub use types::{Art, Cls, Span};

use layout::{layout_class, layout_flowchart, layout_grouped, CanvasResult};
use layout_seq::layout_sequence;
use parse::{parse_class, parse_er, parse_graph, parse_sequence, parse_state};

/// Render a Mermaid source block as Unicode box-drawing art.
///
/// Supported: `graph`/`flowchart` (including `subgraph`), `stateDiagram`,
/// `classDiagram`, `erDiagram` and `sequenceDiagram`.
///
/// The diagram is laid out at whatever size it needs; `art.width` reports how
/// many columns that turned out to be. Deciding what to do when that exceeds
/// the space at hand is the caller's — [`source_box`] is the usual answer:
///
/// ```
/// # let src = "flowchart LR\n  A --> B";
/// # let cols = 80;
/// let art = micro_mermaid::render(src);
/// let shown = match &art {
///     Some(a) if a.width <= cols => a,
///     _ => { micro_mermaid::source_box(src, cols); return; }
/// };
/// # let _ = shown;
/// ```
///
/// `None` means there is no art to show: blank input, a syntax error, a
/// diagram type this renderer does not draw, or one large enough that laying
/// it out is refused. `diagram_kind` separates the middle two.
///
/// Rendering is best-effort. A flowchart keeps whatever parsed; the stricter
/// grammars additionally get one retry without their final line, which is
/// what keeps a streaming diagram on screen while its last statement is
/// half-typed. Everything given up on is listed in `art.warnings` —
/// advisory only, never a reason to withhold the art.
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
///
/// State, class, ER and sequence fail a whole diagram on one unreadable
/// statement, and while a source is streaming its last line is usually still
/// being typed — so without this a diagram alternates with the source box all
/// the way in. Only the final line is dropped, and doing so is always
/// reported, so a finished document with a bad last line still says what it
/// lost rather than quietly rendering short.
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
        // A pie reads its own source: it is rows of values rather than a graph, and has
        // nothing to gain from the node-and-edge model the others share.
        DiagramKind::Pie => plain(pie::render_pie(src)),
        // A mind map is written as indentation rather than as edges, so it reads its own
        // source too.
        DiagramKind::Mindmap => plain(mindmap::render_mindmap(src)),
        DiagramKind::Timeline => plain(timeline::render_timeline(src)),
        DiagramKind::Journey => plain(journey::render_journey(src)),
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
