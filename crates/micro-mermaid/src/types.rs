//! The public shape of a rendered diagram.

/// Semantic class of a run of cells. The renderer never knows about colour;
/// consumers map these to their own theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cls {
    /// Box outlines, subgraph frames, compartment rules.
    Border,
    /// Node / participant / compartment labels.
    Text,
    /// Connector lines and arrowheads.
    Edge,
    /// Text sitting on an edge.
    EdgeLabel,
    /// The `mermaid: <kind>` header of a source box.
    Title,
    /// Blank filler.
    None,
}

/// A run of adjacent cells sharing one semantic class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub cls: Cls,
}

/// A rendered diagram. `plain[i]` and `styled[i]` describe the same row:
/// `plain` is right-trimmed for display width and copy/paste, `styled` keeps
/// the run structure needed to colour it.
///
/// `width` is the display columns the widest row needs — the number to compare
/// against the space available. It cannot be recovered from `plain`, whose rows
/// are strings of code points, not columns.
///
/// `warnings` lists source the flowchart grammar could not read and dropped.
/// Non-empty means the art is real but incomplete — some of what was written is
/// not in it. Only flowcharts warn; the other grammars refuse the whole diagram
/// instead, and `render` returns `None`.
///
/// They are advisory. Do not gate rendering on them: the art is the best drawing
/// of the source either way, and a diagram being typed or streamed warns at
/// nearly every intermediate state. Show them alongside, or once it settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Art {
    pub plain: Vec<String>,
    pub styled: Vec<Vec<Span>>,
    pub width: usize,
    pub warnings: Vec<String>,
}
