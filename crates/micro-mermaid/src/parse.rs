//! Source text to diagram model.
//!
//! Every `parse_x` returns `None` when the source is not that kind of diagram,
//! or when it exceeds a cap — `render` tries each in turn and falls back to a
//! framed copy of the source when they all decline.

use std::collections::HashMap;

use crate::graph::{
    empty_class_info, parse_dir, ClassInfo, Edge, Graph, Group, Head, LineKind, Shape, MAX_EDGES,
    MAX_GROUPS, MAX_GROUP_DEPTH, MAX_MEMBERS, MAX_NODES,
};
use crate::labels::{
    ascii_lower, clean_label, decode_html_entities, display_generics, is_id_char, src_lines,
};

// ---------------------------------------------------------------- statements

fn flush_statement(cur: &str, out: &mut Vec<String>) {
    let trimmed = cur.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
}

/// Split one source line into statements on `;`, stopping at a `%%` comment.
///
/// Quoted spans are opaque, so a label may contain `;` and `%%`.
pub fn split_statements(line: &str, out: &mut Vec<String>) {
    let chars: Vec<char> = line.chars().collect();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_quotes {
            if c == '"' {
                in_quotes = false;
            }
            cur.push(c);
        } else if c == '"' {
            in_quotes = true;
            cur.push(c);
        } else if c == '%' && chars.get(i + 1) == Some(&'%') {
            break;
        } else if c == ';' {
            flush_statement(&cur, out);
            cur.clear();
        } else {
            cur.push(c);
        }
        i += 1;
    }
    flush_statement(&cur, out);
}

/// All statements in a source block, in order.
pub fn statements_of(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src_lines(src) {
        split_statements(&line, &mut out);
    }
    out
}

fn first_word(s: &str) -> String {
    s.split_whitespace().next().unwrap_or("").to_string()
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(|w| w.to_string()).collect()
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Diagram kind from the header statement, lowercased.
fn header_kind(statements: &[String]) -> Option<String> {
    let header = statements.first()?;
    let kind = first_word(header);
    if kind.is_empty() {
        None
    } else {
        Some(ascii_lower(&kind))
    }
}

/// A diagram type this renderer draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagramKind {
    Flowchart,
    State,
    Class,
    Er,
    Sequence,
    Pie,
    Mindmap,
    Timeline,
    Journey,
    Gantt,
    Quadrant,
    Requirement,
}

/// The kind of diagram `src` declares, or `None` if its header names no type
/// this renderer draws.
///
/// Reads the header only — it says nothing about whether the body parses.
/// Pair it with `render` to tell a source this renderer will never draw from
/// one that is merely malformed: `render(src).is_none() && diagram_kind(src).is_some()`
/// means a syntax error.
///
/// Each branch mirrors the header test in the matching `parse_x`, so the two
/// always agree on what they recognise.
pub fn diagram_kind(src: &str) -> Option<DiagramKind> {
    let statements = statements_of(src);
    let kind = header_kind(&statements)?;
    if kind == "graph" || kind == "flowchart" {
        Some(DiagramKind::Flowchart)
    } else if kind.starts_with("statediagram") {
        Some(DiagramKind::State)
    } else if kind.starts_with("classdiagram") {
        Some(DiagramKind::Class)
    } else if kind == "erdiagram" {
        Some(DiagramKind::Er)
    } else if kind == "sequencediagram" {
        Some(DiagramKind::Sequence)
    } else if kind == "pie" {
        Some(DiagramKind::Pie)
    } else if kind == "mindmap" {
        Some(DiagramKind::Mindmap)
    } else if kind == "timeline" {
        Some(DiagramKind::Timeline)
    } else if kind == "journey" {
        Some(DiagramKind::Journey)
    } else if kind == "gantt" {
        Some(DiagramKind::Gantt)
    } else if kind == "quadrantchart" {
        Some(DiagramKind::Quadrant)
    } else if kind == "requirementdiagram" {
        Some(DiagramKind::Requirement)
    } else {
        None
    }
}

// ----------------------------------------------------------------- flowchart

pub fn parse_graph(src: &str) -> Option<Graph> {
    let statements = statements_of(src);
    let kind = header_kind(&statements);
    if kind.as_deref() != Some("graph") && kind.as_deref() != Some("flowchart") {
        return None;
    }

    let header_words = words(&statements[0]);
    let dir_token = header_words.get(1).map(|s| s.as_str()).unwrap_or("TB");
    let mut graph = Graph::new(parse_dir(dir_token));
    let mut stack: Vec<usize> = Vec::new();

    for st in &statements[1..] {
        let first = ascii_lower(&first_word(st));
        match first.as_str() {
            "subgraph" => {
                if graph.groups.len() >= MAX_GROUPS || stack.len() >= MAX_GROUP_DEPTH {
                    return None;
                }
                let rest = st["subgraph".len()..].trim();
                let (id, label) = parse_subgraph_decl(rest);
                let parent = stack.last().copied();
                graph.groups.push(Group { id, label, parent });
                stack.push(graph.groups.len() - 1);
                graph.cur_group = stack.last().copied();
                continue;
            }
            "end" => {
                stack.pop();
                graph.cur_group = stack.last().copied();
                continue;
            }
            "classdef" | "class" | "style" | "linkstyle" | "click" | "direction" => continue,
            _ => {}
        }
        parse_statement(st, &mut graph);
        if graph.over_cap {
            return None;
        }
    }

    if graph.nodes.is_empty() {
        None
    } else {
        Some(graph)
    }
}

/// `subgraph id[Title]`, `subgraph "Title"`, or a bare title.
fn parse_subgraph_decl(rest: &str) -> (String, String) {
    if let Some(after_quote) = rest.strip_prefix('"') {
        if let Some(close) = after_quote.find('"') {
            let label = &after_quote[..close];
            return (label.to_string(), decode_html_entities(label));
        }
    }
    if let Some(open) = rest.find('[') {
        let id = rest[..open].trim();
        let label = clean_label(rest[open + 1..].trim_end_matches(']').trim());
        if !id.is_empty() && !label.is_empty() {
            return (id.to_string(), label);
        }
    }
    (rest.to_string(), rest.to_string())
}

/// A chain of `node link node link node ...`, each link fanning out over `&`.
///
/// Parses as far as it can and keeps the prefix, matching upstream mermaid.js.
/// Whatever it could not read is recorded in `graph.warnings` rather than
/// failing the diagram — see the note on that field.
fn parse_statement(st: &str, graph: &mut Graph) {
    let chars: Vec<char> = st.chars().collect();
    let mut i;

    let head = match parse_node_group(&chars, 0, graph) {
        Some(h) => h,
        None => {
            graph
                .warnings
                .push(format!("dropped, does not start with a node: \"{st}\""));
            return;
        }
    };
    let mut prev = head.group;
    i = head.next;

    loop {
        i = skip_spaces(&chars, i);
        if i >= chars.len() {
            break;
        }
        let link = match parse_link(&chars, i) {
            Some(l) => l,
            None => {
                let rest: String = chars[i..].iter().collect();
                graph
                    .warnings
                    .push(format!("dropped, expected a link: \"{rest}\""));
                break;
            }
        };
        i = skip_spaces(&chars, link.next);
        let target = match parse_node_group(&chars, i, graph) {
            Some(t) => t,
            None => {
                graph
                    .warnings
                    .push(format!("dropped, link has no target: \"{st}\""));
                break;
            }
        };
        i = target.next;
        let mut aborted = false;
        'edges: for &f in &prev {
            for &t in &target.group {
                // `A <-- B` reads right-to-left: swap the endpoints so the arrow that
                // was written on the left becomes a normal forward head.
                let reversed = link.left == Head::Arrow && link.right != Head::Arrow;
                let pushed = graph.push_edge(Edge {
                    from: if reversed { t } else { f },
                    to: if reversed { f } else { t },
                    label: link.label.clone(),
                    head_to: if reversed { Head::Arrow } else { link.right },
                    head_from: if reversed { link.right } else { link.left },
                    line: link.line,
                });
                if !pushed {
                    aborted = true;
                    break 'edges;
                }
            }
        }
        if aborted {
            return;
        }
        prev = target.group;
    }
}

struct NodeGroup {
    group: Vec<usize>,
    next: usize,
}

/// One or more nodes joined by `&`, which fan out into a cross product.
fn parse_node_group(chars: &[char], start: usize, graph: &mut Graph) -> Option<NodeGroup> {
    let first = parse_node(chars, start, graph)?;
    let mut group = vec![first.index];
    let mut i = first.next;
    loop {
        let j = skip_spaces(chars, i);
        if chars.get(j) != Some(&'&') {
            break;
        }
        let next = parse_node(chars, j + 1, graph)?;
        group.push(next.index);
        i = next.next;
    }
    Some(NodeGroup { group, next: i })
}

fn skip_spaces(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    i
}

struct NodeRef {
    index: usize,
    next: usize,
}

fn parse_node(chars: &[char], start: usize, graph: &mut Graph) -> Option<NodeRef> {
    let mut i = skip_spaces(chars, start);
    let id_start = i;
    while i < chars.len() && is_id_char(chars[i]) {
        i += 1;
    }
    if i == id_start {
        return None;
    }
    let id: String = chars[id_start..i].iter().collect();

    let shaped = read_shape_at(chars, i);
    if let Some(unclosed) = shaped.unclosed {
        graph.warnings.push(format!(
            "node \"{id}\": label is missing its closing `{unclosed}`"
        ));
    }
    let index = graph.node_index(&id, shaped.label.as_deref(), shaped.shape)?;
    Some(NodeRef {
        index,
        next: shaped.after,
    })
}

/// What a shape bracket yielded. `unclosed` is set when the bracket never closed.
struct Shaped {
    shape: Shape,
    label: Option<String>,
    after: usize,
    /// The closing token that was expected but never found.
    unclosed: Option<&'static str>,
}

/// Dispatch on the bracket following an id to pick shape and closing token.
fn read_shape_at(chars: &[char], i: usize) -> Shaped {
    let c = chars.get(i).copied();
    let n = chars.get(i + 1).copied();
    match c {
        Some('[') => match n {
            Some('[') => read_shape(chars, i + 2, "]]", Shape::Rect),
            Some('(') => read_shape(chars, i + 2, ")]", Shape::Round),
            _ => read_shape(chars, i + 1, "]", Shape::Rect),
        },
        Some('(') => match n {
            Some('(') => read_shape(chars, i + 2, "))", Shape::Round),
            Some('[') => read_shape(chars, i + 2, "])", Shape::Round),
            _ => read_shape(chars, i + 1, ")", Shape::Round),
        },
        Some('{') => match n {
            Some('{') => read_shape(chars, i + 2, "}}", Shape::Diamond),
            _ => read_shape(chars, i + 1, "}", Shape::Diamond),
        },
        Some('>') => read_shape(chars, i + 1, "]", Shape::Rect),
        _ => Shaped {
            shape: Shape::Rect,
            label: None,
            after: i,
            unclosed: None,
        },
    }
}

/// Read label text up to `closer`.
///
/// Quoting is decided by the first non-space character: inside a quoted label
/// the closer is ignored until the quote closes, so `A["a] b"]` is one node.
/// An unquoted label ends at the first closer, so `A[5" pipe]` keeps its quote.
fn read_shape(chars: &[char], start: usize, closer: &'static str, shape: Shape) -> Shaped {
    let mut j = start;
    while matches!(chars.get(j), Some(&' ') | Some(&'\t')) {
        j += 1;
    }
    let quoted = chars.get(j) == Some(&'"');

    let closer_chars: Vec<char> = closer.chars().collect();
    let mut i = start;
    let mut text = String::new();
    let mut in_quotes = false;
    while i < chars.len() {
        let c = chars[i];
        if quoted && c == '"' {
            in_quotes = !in_quotes;
            text.push(c);
            i += 1;
            continue;
        }
        if !in_quotes && chars[i..].starts_with(closer_chars.as_slice()) {
            return Shaped {
                shape,
                label: Some(clean_label(&text)),
                after: i + closer_chars.len(),
                unclosed: None,
            };
        }
        text.push(c);
        i += 1;
    }
    // Ran off the end still looking for the closer: everything after the opening
    // bracket became label text, so any link operator in it was swallowed.
    Shaped {
        shape,
        label: Some(clean_label(&text)),
        after: chars.len(),
        unclosed: Some(closer),
    }
}

fn is_link_char(c: char) -> bool {
    matches!(c, '-' | '.' | '=' | '<' | '>')
}

struct Link {
    left: Head,
    right: Head,
    line: LineKind,
    label: Option<String>,
    next: usize,
}

struct Trailing {
    head: Head,
    next: usize,
}

/// A trailing `o`/`x` head, only when followed by a statement boundary.
fn trailing_head(chars: &[char], i: usize) -> Option<Trailing> {
    let head = match chars.get(i) {
        Some(&'o') => Head::Circle,
        Some(&'x') => Head::Cross,
        _ => return None,
    };
    let after = chars.get(i + 1);
    let boundary = matches!(
        after,
        None | Some(&' ') | Some(&'\t') | Some(&'|') | Some(&'&') | Some(&';')
    );
    if boundary {
        Some(Trailing { head, next: i + 1 })
    } else {
        None
    }
}

fn line_kind(op: &str) -> LineKind {
    if op.contains('=') {
        LineKind::Thick
    } else if op.contains('.') {
        LineKind::Dotted
    } else {
        LineKind::Solid
    }
}

/// Read a link operator and its label.
///
/// Labels come in two forms: `-->|text|` and the inline `-- text -->`, the
/// latter only when the first operator carried no head.
fn parse_link(chars: &[char], start: usize) -> Option<Link> {
    let mut i = skip_spaces(chars, start);
    let mut left = Head::None;
    // A leading `o`/`x` decorates the tail, but only directly before an operator.
    if matches!(chars.get(i), Some(&'o') | Some(&'x'))
        && matches!(chars.get(i + 1), Some(&'-') | Some(&'.') | Some(&'='))
    {
        left = if chars[i] == 'o' {
            Head::Circle
        } else {
            Head::Cross
        };
        i += 1;
    }

    let op_start = i;
    while i < chars.len() && is_link_char(chars[i]) {
        i += 1;
    }
    if i == op_start {
        return None;
    }
    let op1: String = chars[op_start..i].iter().collect();
    if left == Head::None && op1.starts_with('<') {
        left = Head::Arrow;
    }

    let mut line = line_kind(&op1);
    let mut right = if op1.contains('>') {
        Head::Arrow
    } else {
        Head::None
    };
    if right == Head::None {
        if let Some(trailing) = trailing_head(chars, i) {
            right = trailing.head;
            i = trailing.next;
        }
    }

    if chars.get(i) == Some(&'|') {
        i += 1;
        let l_start = i;
        while i < chars.len() && chars[i] != '|' {
            i += 1;
        }
        let label = clean_label(&chars[l_start..i].iter().collect::<String>());
        if chars.get(i) == Some(&'|') {
            i += 1;
        }
        return Some(Link {
            left,
            right,
            line,
            label: non_empty(label),
            next: i,
        });
    }

    if right == Head::None {
        let text_start = skip_spaces(chars, i);
        let mut j = text_start;
        while j < chars.len() && !is_link_char(chars[j]) {
            j += 1;
        }
        if j < chars.len() && j > text_start && chars[j] != '<' {
            let text: String = chars[text_start..j].iter().collect();
            let op2_start = j;
            while j < chars.len() && is_link_char(chars[j]) {
                j += 1;
            }
            let op2: String = chars[op2_start..j].iter().collect();
            if op2.contains('>') {
                right = Head::Arrow;
            } else if let Some(trailing) = trailing_head(chars, j) {
                right = trailing.head;
                j = trailing.next;
            }
            if line == LineKind::Solid {
                line = line_kind(&op2);
            }
            return Some(Link {
                left,
                right,
                line,
                label: non_empty(clean_label(&text)),
                next: j,
            });
        }
    }

    Some(Link {
        left,
        right,
        line,
        label: None,
        next: i,
    })
}

// --------------------------------------------------------------------- state

pub fn parse_state(src: &str) -> Option<Graph> {
    let statements = statements_of(src);
    let kind = header_kind(&statements)?;
    if !kind.starts_with("statediagram") {
        return None;
    }

    let mut graph = Graph::default();
    let mut in_note = false;

    for st in &statements[1..] {
        if in_note {
            if ascii_lower(st) == "end note" {
                in_note = false;
            }
            continue;
        }
        let first = ascii_lower(&first_word(st));
        if first == "direction" {
            let w = words(st);
            graph.dir = parse_dir(w.get(1).map(|s| s.as_str()).unwrap_or(""));
        } else if first == "note" {
            // A single-line `note ... : text` needs no terminator.
            if !st.contains(':') {
                in_note = true;
            }
        } else if first == "state" {
            parse_state_decl(st, &mut graph)?;
        } else if matches!(
            first.as_str(),
            "classdef" | "class" | "hide" | "scale" | "}" | "--"
        ) {
            // Styling and composite-state punctuation carry no layout meaning.
        } else if st.contains("-->") {
            parse_transition(st, &mut graph)?;
        } else {
            parse_state_desc(st, &mut graph)?;
        }
        if graph.over_cap {
            return None;
        }
    }

    if graph.nodes.is_empty() {
        None
    } else {
        Some(graph)
    }
}

/// `state "Label" as id`, `state id <<choice>>`, or `state id {`.
fn parse_state_decl(st: &str, graph: &mut Graph) -> Option<()> {
    let rest = st["state".len()..].trim();
    let rest = rest.strip_suffix('{').unwrap_or(rest).trim();
    if rest.is_empty() {
        return Some(());
    }

    if let Some(after_quote) = rest.strip_prefix('"') {
        let close = after_quote.find('"')?;
        let label = &after_quote[..close];
        let after = after_quote[close + 1..].trim();
        let id = after.strip_prefix("as").map(|s| s.trim()).unwrap_or(label);
        return graph
            .node_label(id, &decode_html_entities(label))
            .map(|_| ());
    }

    let mut shape = Shape::Round;
    let mut id = rest;
    let mut stereotyped = false;
    if let Some(pos) = rest.find("<<") {
        let stereo = rest[pos + 2..]
            .strip_suffix(">>")
            .unwrap_or(&rest[pos + 2..])
            .trim();
        if stereo == "choice" {
            shape = Shape::Diamond;
        }
        id = rest[..pos].trim();
        stereotyped = true;
    }
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return None;
    }
    graph
        .node_index(id, if stereotyped { Some(id) } else { None }, shape)
        .map(|_| ())
}

/// `A --> B: label`, including chains `A --> B --> C`.
fn parse_transition(st: &str, graph: &mut Graph) -> Option<()> {
    let mut rest = st;
    let mut prev: Option<usize> = None;

    while let Some((lhs, rhs)) = rest.split_once("-->") {
        let from_id = lhs.trim_end().trim_end_matches('-').trim();
        let from = if let Some(p) = prev {
            // Mid-chain: the source is the previous target, so nothing may precede.
            if !from_id.is_empty() {
                return None;
            }
            p
        } else {
            if from_id.is_empty() {
                return None;
            }
            state_endpoint(graph, from_id, true)?
        };

        let next_arrow = rhs.find("-->");
        let (to_part_raw, tail) = match next_arrow {
            Some(pos) => (&rhs[..pos], &rhs[pos..]),
            None => (rhs, ""),
        };

        let colon = to_part_raw.split_once(':');
        let to_part = colon.map(|(a, _)| a).unwrap_or(to_part_raw);
        let label = colon.and_then(|(_, l)| non_empty(decode_html_entities(l.trim())));

        let to_id = to_part
            .trim_start()
            .trim_start_matches('>')
            .trim_end()
            .trim_end_matches('-')
            .trim();
        if to_id.is_empty() {
            return None;
        }
        let to = state_endpoint(graph, to_id, false)?;

        if !graph.push_edge(Edge {
            from,
            to,
            label,
            head_to: Head::Arrow,
            head_from: Head::None,
            line: LineKind::Solid,
        }) {
            return Some(());
        }
        prev = Some(to);
        rest = tail;
    }
    Some(())
}

/// `[*]` is start or end depending on which side of the arrow it sits.
fn state_endpoint(graph: &mut Graph, id: &str, is_source: bool) -> Option<usize> {
    if id == "[*]" {
        let key = if is_source { "[*]start" } else { "[*]end" };
        graph.node_index(key, Some("●"), Shape::Round)
    } else {
        graph.node_index(id, None, Shape::Round)
    }
}

/// `id: description`, or a bare state name.
fn parse_state_desc(st: &str, graph: &mut Graph) -> Option<()> {
    if let Some((id_part, desc_part)) = st.split_once(':') {
        let id = id_part.trim();
        let desc = desc_part.trim();
        if id.is_empty() || id.chars().any(char::is_whitespace) || desc.is_empty() {
            return None;
        }
        return graph
            .node_label(id, &decode_html_entities(desc))
            .map(|_| ());
    }
    if st.chars().any(char::is_whitespace) {
        return None;
    }
    graph.node_index(st, None, Shape::Round).map(|_| ())
}

// --------------------------------------------------------------------- class

/// Relation operators, longest-first so `--|>` wins over `--`.
const CLASS_OPS: &[(&str, Head, Head, LineKind)] = &[
    ("<|--", Head::Triangle, Head::None, LineKind::Solid),
    ("--|>", Head::None, Head::Triangle, LineKind::Solid),
    ("<|..", Head::Triangle, Head::None, LineKind::Dotted),
    ("..|>", Head::None, Head::Triangle, LineKind::Dotted),
    ("*--", Head::DiamondFill, Head::None, LineKind::Solid),
    ("--*", Head::None, Head::DiamondFill, LineKind::Solid),
    ("o--", Head::DiamondOpen, Head::None, LineKind::Solid),
    ("--o", Head::None, Head::DiamondOpen, LineKind::Solid),
    ("<--", Head::Arrow, Head::None, LineKind::Solid),
    ("-->", Head::None, Head::Arrow, LineKind::Solid),
    ("<..", Head::Arrow, Head::None, LineKind::Dotted),
    ("..>", Head::None, Head::Arrow, LineKind::Dotted),
    ("--", Head::None, Head::None, LineKind::Solid),
    ("..", Head::None, Head::None, LineKind::Dotted),
];

const MAX_CLASS_OP: usize = 4;

fn sync_infos(graph: &Graph, infos: &mut Vec<ClassInfo>) {
    while infos.len() < graph.nodes.len() {
        infos.push(empty_class_info());
    }
}

/// Declare a class, keeping `infos` aligned with `graph.nodes`.
fn declare_class(graph: &mut Graph, infos: &mut Vec<ClassInfo>, name: &str) -> Option<usize> {
    let idx = graph.node_index(name, None, Shape::Rect);
    sync_infos(graph, infos);
    idx
}

pub fn parse_class(src: &str) -> Option<(Graph, Vec<ClassInfo>)> {
    let statements = statements_of(src);
    let kind = header_kind(&statements)?;
    if !kind.starts_with("classdiagram") {
        return None;
    }

    let mut graph = Graph::default();
    let mut infos: Vec<ClassInfo> = Vec::new();
    let mut cur_class: Option<usize> = None;

    for st in &statements[1..] {
        if let Some(ci) = cur_class {
            if st == "}" {
                cur_class = None;
            } else {
                push_member(&mut infos[ci], st);
            }
            continue;
        }

        let first = ascii_lower(&first_word(st));
        if first == "direction" {
            let w = words(st);
            graph.dir = parse_dir(w.get(1).map(|s| s.as_str()).unwrap_or(""));
            continue;
        }
        if matches!(
            first.as_str(),
            "note"
                | "callback"
                | "click"
                | "link"
                | "style"
                | "cssclass"
                | "classdef"
                | "namespace"
                | "}"
        ) {
            continue;
        }
        if first == "class" {
            let rest = st["class".len()..].trim();
            let open = rest.ends_with('{');
            let name = if open {
                rest[..rest.len() - 1].trim()
            } else {
                rest
            };
            if name.is_empty() || name.chars().any(char::is_whitespace) {
                return None;
            }
            let idx = declare_class(&mut graph, &mut infos, name)?;
            if open {
                cur_class = Some(idx);
            }
            continue;
        }

        if let Some(after) = st.strip_prefix("<<") {
            let (annotation, rest) = after.split_once(">>")?;
            let name = rest.trim();
            if name.is_empty() || name.chars().any(char::is_whitespace) {
                return None;
            }
            let idx = declare_class(&mut graph, &mut infos, name)?;
            infos[idx].annotation = Some(annotation.trim().to_string());
            continue;
        }

        if let Some(rel) = parse_class_relation(st) {
            let f = declare_class(&mut graph, &mut infos, &rel.from)?;
            let t = declare_class(&mut graph, &mut infos, &rel.to)?;
            if graph.edges.len() >= MAX_EDGES {
                return None;
            }
            graph.edges.push(Edge {
                from: f,
                to: t,
                label: rel.label,
                head_to: rel.head_to,
                head_from: rel.head_from,
                line: rel.line,
            });
            continue;
        }

        if let Some((id_part, text_part)) = st.split_once(':') {
            let id = id_part.trim();
            let text = text_part.trim();
            if id.is_empty() || id.chars().any(char::is_whitespace) || text.is_empty() {
                return None;
            }
            let idx = declare_class(&mut graph, &mut infos, id)?;
            push_member(&mut infos[idx], text);
            continue;
        }
        return None;
    }

    if graph.nodes.is_empty() {
        return None;
    }
    sync_infos(&graph, &mut infos);
    Some((graph, infos))
}

/// Add a member to the attribute or method compartment, eliding past the cap.
pub fn push_member(info: &mut ClassInfo, raw: &str) {
    if let Some(after) = raw.strip_prefix("<<") {
        if let Some((annotation, _)) = after.split_once(">>") {
            info.annotation = Some(annotation.trim().to_string());
        }
        return;
    }
    let member = decode_html_entities(&display_generics(raw.trim()));
    let list = if member.contains('(') {
        &mut info.methods
    } else {
        &mut info.attrs
    };
    if list.len() < MAX_MEMBERS {
        list.push(member);
    } else if list.len() == MAX_MEMBERS {
        list.push("…".to_string());
    }
}

struct ClassRelation {
    from: String,
    to: String,
    head_from: Head,
    head_to: Head,
    line: LineKind,
    label: Option<String>,
}

fn parse_class_relation(st: &str) -> Option<ClassRelation> {
    let chars: Vec<char> = st.chars().collect();
    let mut found: Option<(usize, usize, Head, Head, LineKind)> = None;

    'outer: for pos in 0..chars.len() {
        let end = (pos + MAX_CLASS_OP).min(chars.len());
        let tail: String = chars[pos..end].iter().collect();
        for &(op, head_from, head_to, line) in CLASS_OPS {
            if !tail.starts_with(op) {
                continue;
            }
            // `o` is also an identifier character: skip a match glued to a name.
            if op.starts_with('o') && pos > 0 && is_id_char(chars[pos - 1]) {
                continue;
            }
            let op_len = op.chars().count();
            if op.ends_with('o') {
                if let Some(&after) = chars.get(pos + op_len) {
                    if is_id_char(after) {
                        continue;
                    }
                }
            }
            found = Some((pos, op_len, head_from, head_to, line));
            break 'outer;
        }
    }
    let (pos, op_len, head_from, head_to, line) = found?;

    let lhs_raw: String = chars[..pos].iter().collect();
    let lhs_raw = lhs_raw.trim().to_string();
    let rhs_raw: String = chars[pos + op_len..].iter().collect();
    let rhs_raw = rhs_raw.trim().to_string();

    let (lhs, card_from) = strip_cardinality_suffix(&lhs_raw);
    let (rhs, card_to) = strip_cardinality_prefix(&rhs_raw);

    let split = rhs.split_once(':');
    let to_id = split.map(|(a, _)| a).unwrap_or(&rhs).trim().to_string();
    let rel_label = split.and_then(|(_, l)| non_empty(decode_html_entities(l.trim())));

    if lhs.is_empty()
        || to_id.is_empty()
        || lhs.chars().any(char::is_whitespace)
        || to_id.chars().any(char::is_whitespace)
    {
        return None;
    }

    let label = non_empty(
        [
            card_from.as_str(),
            rel_label.as_deref().unwrap_or(""),
            card_to.as_str(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" "),
    );
    Some(ClassRelation {
        from: lhs,
        to: to_id,
        head_from,
        head_to,
        line,
        label,
    })
}

/// `Class "1"` — a quoted cardinality trailing the left-hand name.
fn strip_cardinality_suffix(s: &str) -> (String, String) {
    let t = s.trim_end();
    if let Some(rest) = t.strip_suffix('"') {
        if let Some(q) = rest.rfind('"') {
            return (rest[..q].trim_end().to_string(), rest[q + 1..].to_string());
        }
    }
    (t.to_string(), String::new())
}

/// `"0..*" Class` — a quoted cardinality leading the right-hand name.
fn strip_cardinality_prefix(s: &str) -> (String, String) {
    let t = s.trim_start();
    if let Some(rest) = t.strip_prefix('"') {
        if let Some(q) = rest.find('"') {
            return (
                rest[q + 1..].trim_start().to_string(),
                rest[..q].to_string(),
            );
        }
    }
    (t.to_string(), String::new())
}

// ------------------------------------------------------------------------ ER

pub fn parse_er(src: &str) -> Option<(Graph, Vec<ClassInfo>)> {
    let statements = statements_of(src);
    if header_kind(&statements).as_deref() != Some("erdiagram") {
        return None;
    }

    let mut graph = Graph::default();
    let mut infos: Vec<ClassInfo> = Vec::new();
    let mut cur_entity: Option<usize> = None;

    for st in &statements[1..] {
        if let Some(ce) = cur_entity {
            if st == "}" {
                cur_entity = None;
            } else {
                push_er_attribute(&mut infos[ce], st);
            }
            continue;
        }

        if let Some(rel) = split_er_relationship(st) {
            let tokens = words(&rel.rel);
            if tokens.len() != 3 {
                return None;
            }
            let op = parse_er_op(&tokens[1])?;
            let f = er_entity(&mut graph, &mut infos, &tokens[0])?;
            let t = er_entity(&mut graph, &mut infos, &tokens[2])?;
            if graph.edges.len() >= MAX_EDGES {
                return None;
            }
            let rel_label = rel.label.as_deref().map(clean_label).unwrap_or_default();
            let label = non_empty(
                [op.card_l.as_str(), rel_label.as_str(), op.card_r.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            graph.edges.push(Edge {
                from: f,
                to: t,
                label,
                head_to: Head::None,
                head_from: Head::None,
                line: op.line,
            });
            continue;
        }

        let open = st.ends_with('{');
        let decl = if open {
            st[..st.len() - 1].trim()
        } else {
            st.as_str()
        };
        if decl.is_empty() || words(decl).len() != 1 {
            return None;
        }
        let idx = er_entity(&mut graph, &mut infos, decl)?;
        if open {
            cur_entity = Some(idx);
        }
    }

    if graph.nodes.is_empty() {
        return None;
    }
    while infos.len() < graph.nodes.len() {
        infos.push(empty_class_info());
    }
    Some((graph, infos))
}

fn er_entity(graph: &mut Graph, infos: &mut Vec<ClassInfo>, token: &str) -> Option<usize> {
    let idx = if let Some(open) = token.find('[') {
        let id = &token[..open];
        let label = clean_label(token[open + 1..].trim_end_matches(']'));
        if id.is_empty() || label.is_empty() {
            return None;
        }
        graph.node_label(id, &label)?
    } else {
        graph.node_index(token, None, Shape::Rect)?
    };
    while infos.len() < graph.nodes.len() {
        infos.push(empty_class_info());
    }
    Some(idx)
}

struct ErRelationship {
    rel: String,
    label: Option<String>,
}

fn split_er_relationship(st: &str) -> Option<ErRelationship> {
    let split = st.split_once(':');
    let rel = split.map(|(a, _)| a).unwrap_or(st);
    let label = split.map(|(_, b)| b.trim().to_string());
    if words(rel).iter().any(|t| parse_er_op(t).is_some()) {
        Some(ErRelationship {
            rel: rel.to_string(),
            label,
        })
    } else {
        None
    }
}

struct ErOp {
    card_l: String,
    card_r: String,
    line: LineKind,
}

/// A crow's-foot operator: two cardinality glyphs around `--` or `..`.
fn parse_er_op(tok: &str) -> Option<ErOp> {
    if tok.len() != 6 || !tok.is_ascii() {
        return None;
    }
    let mid = &tok[2..4];
    let line = if mid == "--" {
        LineKind::Solid
    } else if mid == ".." {
        LineKind::Dotted
    } else {
        return None;
    };
    let card_l = er_card(&tok[0..2])?;
    let card_r = er_card(&tok[4..6])?;
    Some(ErOp {
        card_l,
        card_r,
        line,
    })
}

fn er_card(tok: &str) -> Option<String> {
    match tok {
        "|o" | "o|" => Some("0..1".to_string()),
        "||" => Some("1".to_string()),
        "}o" | "o{" => Some("0..*".to_string()),
        "}|" | "|{" => Some("1..*".to_string()),
        _ => None,
    }
}

/// ER attributes are `type name`; a trailing quoted comment is dropped.
pub fn push_er_attribute(info: &mut ClassInfo, raw: &str) {
    let mut parts: Vec<String> = Vec::new();
    for tok in words(raw) {
        if tok.starts_with('"') {
            break;
        }
        parts.push(tok);
    }
    if parts.is_empty() {
        return;
    }
    let line = decode_html_entities(&parts.join(" "));
    if info.attrs.len() < MAX_MEMBERS {
        info.attrs.push(line);
    } else if info.attrs.len() == MAX_MEMBERS {
        info.attrs.push("…".to_string());
    }
}

// ------------------------------------------------------------------ sequence

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqHead {
    Arrow,
    Cross,
}

/// Message operators, longest-first so `-->>` wins over `-->`.
const SEQ_OPS: &[(&str, bool, SeqHead)] = &[
    ("-->>", true, SeqHead::Arrow),
    ("->>", false, SeqHead::Arrow),
    ("--x", true, SeqHead::Cross),
    ("-x", false, SeqHead::Cross),
    ("--)", true, SeqHead::Arrow),
    ("-)", false, SeqHead::Arrow),
    ("-->", true, SeqHead::Arrow),
    ("->", false, SeqHead::Arrow),
];

const MAX_SEQ_OP: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteAnchor {
    Over { from: usize, to: usize },
    Left { at: usize },
    Right { at: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeqItem {
    Message {
        from: usize,
        to: usize,
        text: Option<String>,
        dashed: bool,
        head: SeqHead,
    },
    Note {
        anchor: NoteAnchor,
        text: String,
    },
    Divider {
        text: String,
    },
}

#[derive(Debug, Default)]
pub struct Sequence {
    pub labels: Vec<String>,
    pub index: HashMap<String, usize>,
    pub items: Vec<SeqItem>,
}

impl Sequence {
    pub fn new() -> Self {
        Sequence::default()
    }

    pub fn participant(&mut self, id: &str, label: Option<&str>) -> Option<usize> {
        if let Some(&existing) = self.index.get(id) {
            if let Some(label) = label {
                self.labels[existing] = label.to_string();
            }
            return Some(existing);
        }
        if self.labels.len() >= MAX_NODES {
            return None;
        }
        self.index.insert(id.to_string(), self.labels.len());
        self.labels.push(label.unwrap_or(id).to_string());
        Some(self.labels.len() - 1)
    }
}

pub fn parse_sequence(src: &str) -> Option<Sequence> {
    let statements = statements_of(src);
    if header_kind(&statements).as_deref() != Some("sequencediagram") {
        return None;
    }

    let mut seq = Sequence::new();
    let mut autonumber = false;
    let mut msg_count = 0u32;
    // One entry per open block; `true` when it draws a divider on `end`.
    let mut blocks: Vec<bool> = Vec::new();

    for st in &statements[1..] {
        let first = first_word(st);
        let lower = ascii_lower(&first);

        if lower == "participant" || lower == "actor" {
            let rest = st[first.len()..].trim();
            if rest.is_empty() {
                return None;
            }
            let as_split = rest.split_once(" as ");
            let (id, label) = match as_split {
                Some((id, label)) => (id.trim(), Some(clean_label(label))),
                None => (rest, None),
            };
            seq.participant(id, label.as_deref())?;
            continue;
        }
        if lower == "autonumber" {
            autonumber = true;
            continue;
        }
        if matches!(
            lower.as_str(),
            "activate"
                | "deactivate"
                | "create"
                | "destroy"
                | "title"
                | "acctitle"
                | "accdescr"
                | "links"
                | "link"
                | "properties"
        ) {
            continue;
        }
        if lower == "note" {
            let rest = st[first.len()..].trim();
            let note = parse_note_anchor(rest, &mut seq)?;
            if seq.items.len() >= MAX_EDGES {
                return None;
            }
            seq.items.push(SeqItem::Note {
                anchor: note.anchor,
                text: note.text,
            });
            continue;
        }
        if matches!(
            lower.as_str(),
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "else" | "and" | "option"
        ) {
            if matches!(lower.as_str(), "else" | "and" | "option") {
                // A continuation only divides a block that opened one.
                if blocks.last() != Some(&true) {
                    continue;
                }
            } else {
                blocks.push(true);
            }
            if seq.items.len() >= MAX_EDGES {
                return None;
            }
            seq.items.push(SeqItem::Divider {
                text: decode_html_entities(st),
            });
            continue;
        }
        if lower == "rect" || lower == "box" {
            blocks.push(false);
            continue;
        }
        if lower == "end" {
            if blocks.pop() == Some(true) {
                if seq.items.len() >= MAX_EDGES {
                    return None;
                }
                seq.items.push(SeqItem::Divider {
                    text: "end".to_string(),
                });
            }
            continue;
        }

        let msg = parse_seq_message(st, &mut seq)?;
        let mut text = msg.text;
        if autonumber {
            msg_count += 1;
            text = Some(match text {
                None => format!("{msg_count}."),
                Some(t) => format!("{msg_count}. {t}"),
            });
        }
        if seq.items.len() >= MAX_EDGES {
            return None;
        }
        seq.items.push(SeqItem::Message {
            from: msg.from,
            to: msg.to,
            text,
            dashed: msg.dashed,
            head: msg.head,
        });
    }

    if seq.labels.is_empty() {
        None
    } else {
        Some(seq)
    }
}

struct NoteResult {
    text: String,
    anchor: NoteAnchor,
}

fn parse_note_anchor(rest: &str, seq: &mut Sequence) -> Option<NoteResult> {
    enum Kind {
        Over,
        Left,
        Right,
    }

    let lower = ascii_lower(rest);
    let (kind, ids_and_text) = if lower.starts_with("over ") {
        (Kind::Over, &rest[5..])
    } else if lower.starts_with("left of ") {
        (Kind::Left, &rest[8..])
    } else if lower.starts_with("right of ") {
        (Kind::Right, &rest[9..])
    } else {
        return None;
    };

    let (ids_part, text_part) = ids_and_text.split_once(':')?;
    let text = decode_html_entities(text_part.trim());
    let parts: Vec<&str> = ids_part
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let a = seq.participant(parts[0], None)?;

    match kind {
        Kind::Left => Some(NoteResult {
            text,
            anchor: NoteAnchor::Left { at: a },
        }),
        Kind::Right => Some(NoteResult {
            text,
            anchor: NoteAnchor::Right { at: a },
        }),
        Kind::Over => {
            let b = match parts.get(1) {
                Some(&second_name) => seq.participant(second_name, None)?,
                None => a,
            };
            Some(NoteResult {
                text,
                anchor: NoteAnchor::Over {
                    from: a.min(b),
                    to: a.max(b),
                },
            })
        }
    }
}

struct SeqMessage {
    from: usize,
    to: usize,
    text: Option<String>,
    dashed: bool,
    head: SeqHead,
}

fn parse_seq_message(st: &str, seq: &mut Sequence) -> Option<SeqMessage> {
    let chars: Vec<char> = st.chars().collect();
    let mut found: Option<(usize, usize, bool, SeqHead)> = None;

    'outer: for pos in 0..chars.len() {
        let end = (pos + MAX_SEQ_OP).min(chars.len());
        let tail: String = chars[pos..end].iter().collect();
        for &(op, dashed, head) in SEQ_OPS {
            if tail.starts_with(op) {
                found = Some((pos, op.chars().count(), dashed, head));
                break 'outer;
            }
        }
    }
    let (pos, op_len, dashed, head) = found?;

    let from_id: String = chars[..pos].iter().collect();
    let from_id = from_id.trim().to_string();
    if from_id.is_empty() {
        return None;
    }
    // `+`/`-` activate and deactivate the target; they carry no layout meaning.
    let rest_chars: String = chars[pos + op_len..].iter().collect();
    let rest = rest_chars.trim_start().trim_start_matches(['+', '-']);

    let split = rest.split_once(':');
    let to_id = split.map(|(a, _)| a).unwrap_or(rest).trim().to_string();
    let text = split.and_then(|(_, t)| non_empty(decode_html_entities(t.trim())));
    if to_id.is_empty() {
        return None;
    }

    let from = seq.participant(&from_id, None)?;
    let to = seq.participant(&to_id, None)?;
    Some(SeqMessage {
        from,
        to,
        text,
        dashed,
        head,
    })
}
