//! The shared diagram model.

use std::collections::HashMap;

use crate::labels::ascii_upper;

/// Caps that keep layout bounded; exceeding one drops the diagram to fallback.
pub const MAX_NODES: usize = 128;
pub const MAX_EDGES: usize = 512;
pub const MAX_GROUPS: usize = 24;
pub const MAX_GROUP_DEPTH: usize = 6;
/// Class members / ER attributes listed per box before eliding with `…`.
pub const MAX_MEMBERS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Round,
    Diamond,
}

/// Decoration at one end of an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Head {
    None,
    Arrow,
    Circle,
    Cross,
    Triangle,
    DiamondFill,
    DiamondOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Solid,
    Dotted,
    Thick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Down,
    Up,
    Right,
    Left,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub label: String,
    pub shape: Shape,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    pub head_to: Head,
    pub head_from: Head,
    pub line: LineKind,
}

#[derive(Debug, Clone)]
pub struct Group {
    pub id: String,
    pub label: String,
    pub parent: Option<usize>,
}

/// Extra compartment content for class and ER boxes.
#[derive(Debug, Clone, Default)]
pub struct ClassInfo {
    pub annotation: Option<String>,
    pub attrs: Vec<String>,
    pub methods: Vec<String>,
}

pub fn empty_class_info() -> ClassInfo {
    ClassInfo::default()
}

/// `LR`/`RL`/`BT` as written in a header or `direction` statement; else `down`.
pub fn parse_dir(token: &str) -> Dir {
    match ascii_upper(token).as_str() {
        "LR" => Dir::Right,
        "RL" => Dir::Left,
        "BT" => Dir::Up,
        _ => Dir::Down,
    }
}

#[derive(Debug, Clone)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub index: HashMap<String, usize>,
    pub groups: Vec<Group>,
    /// Innermost subgraph each node was declared in, parallel to `nodes`.
    pub node_group: Vec<Option<usize>>,
    pub cur_group: Option<usize>,
    /// Set when a cap was hit; the caller abandons the parse.
    pub over_cap: bool,
    /// Text the flowchart grammar could not read and silently discarded.
    pub warnings: Vec<String>,
    pub dir: Dir,
}

impl Graph {
    pub fn new(dir: Dir) -> Self {
        Graph {
            nodes: Vec::new(),
            edges: Vec::new(),
            index: HashMap::new(),
            groups: Vec::new(),
            node_group: Vec::new(),
            cur_group: None,
            over_cap: false,
            warnings: Vec::new(),
            dir,
        }
    }

    /// Index of `id`, creating the node if new.
    pub fn node_index(&mut self, id: &str, label: Option<&str>, shape: Shape) -> Option<usize> {
        if let Some(&existing) = self.index.get(id) {
            if let Some(label) = label {
                self.nodes[existing].label = label.to_string();
                self.nodes[existing].shape = shape;
            }
            return Some(existing);
        }
        if self.nodes.len() >= MAX_NODES {
            self.over_cap = true;
            return None;
        }
        self.index.insert(id.to_string(), self.nodes.len());
        self.nodes.push(Node {
            label: label.unwrap_or(id).to_string(),
            shape,
        });
        self.node_group.push(self.cur_group);
        Some(self.nodes.len() - 1)
    }

    /// Set a node's label without disturbing its shape, creating it if new.
    pub fn node_label(&mut self, id: &str, label: &str) -> Option<usize> {
        if let Some(&existing) = self.index.get(id) {
            self.nodes[existing].label = label.to_string();
            return Some(existing);
        }
        self.node_index(id, Some(label), Shape::Round)
    }

    /// Append an edge, or flag `over_cap` when `MAX_EDGES` is reached.
    pub fn push_edge(&mut self, edge: Edge) -> bool {
        if self.edges.len() >= MAX_EDGES {
            self.over_cap = true;
            return false;
        }
        self.edges.push(edge);
        true
    }
}

impl Default for Graph {
    fn default() -> Self {
        Graph::new(Dir::Down)
    }
}
