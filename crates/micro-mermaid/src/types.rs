//! The public shape of a rendered diagram.

/// Semantic class of a run of cells.
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

/// A rendered diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Art {
    pub plain: Vec<String>,
    pub styled: Vec<Vec<Span>>,
    pub width: usize,
    pub warnings: Vec<String>,
}
