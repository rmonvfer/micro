//! The raw source in a framed box.
//!
//! What to show when `render` returns `None`, or returns art too wide for the
//! space at hand. Both are the caller's call, so this is theirs to invoke —
//! and theirs to caption, since only they know whether some other view of the
//! diagram exists to point the reader at.

use crate::labels::{src_lines, strip_controls};
use crate::types::{Art, Cls, Span};
use crate::width::{measured, string_width};

fn sat(a: usize, b: usize) -> usize {
    a.saturating_sub(b)
}

/// Frame `src` in a titled box, hard-wrapping its lines to `columns` columns.
///
/// The result can still exceed `columns`: the body wraps to
/// `max(8, columns - 4)` and the ` mermaid: <kind> ` title is never
/// truncated, so a long first token sets a floor. Check `width` if it matters.
pub fn source_box(src: &str, columns: usize) -> Art {
    let src = strip_controls(src);
    let header = src.split_whitespace().next().unwrap_or("diagram");
    let title = format!(" mermaid: {header} ");
    let limit = sat(columns, 4).max(8);

    let mut started = false;
    let mut body: Vec<String> = Vec::new();
    for l in src_lines(&src) {
        let l = l.trim_end();
        if !started && l.is_empty() {
            continue;
        }
        started = true;
        body.extend(chunk_line(l, limit));
    }

    let content_w =
        string_width(&title).max(body.iter().map(|l| string_width(l)).max().unwrap_or(0));
    let inner = content_w + 2;

    let mut plain: Vec<String> = Vec::new();
    let mut styled: Vec<Vec<Span>> = Vec::new();

    let rule = "─".repeat(sat(inner, string_width(&title)));
    plain.push(format!("╭{title}{rule}╮"));
    styled.push(vec![
        Span {
            text: "╭".to_string(),
            cls: Cls::Border,
        },
        Span {
            text: title.clone(),
            cls: Cls::Title,
        },
        Span {
            text: format!("{rule}╮"),
            cls: Cls::Border,
        },
    ]);

    for line in &body {
        let pad = " ".repeat(sat(content_w, string_width(line)));
        plain.push(format!("│ {line}{pad} │"));
        styled.push(vec![
            Span {
                text: "│ ".to_string(),
                cls: Cls::Border,
            },
            Span {
                text: line.clone(),
                cls: Cls::Text,
            },
            Span {
                text: format!("{pad} │"),
                cls: Cls::Border,
            },
        ]);
    }

    let bottom = format!("╰{}╯", "─".repeat(inner));
    plain.push(bottom.clone());
    styled.push(vec![Span {
        text: bottom,
        cls: Cls::Border,
    }]);

    Art {
        plain,
        styled,
        width: inner + 2,
        warnings: Vec::new(),
    }
}

/// Hard-break a line at `limit` columns, never splitting a wide glyph.
fn chunk_line(line: &str, limit: usize) -> Vec<String> {
    if string_width(line) <= limit {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for (c, cw) in measured(line) {
        if cur_w + cw > limit && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push_str(c);
        cur_w += cw;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
