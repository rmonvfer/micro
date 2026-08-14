//! Label and text handling: case folding, HTML/markdown cleanup, entity
//! decoding, and wrapping label text to a box's width.

use crate::width::{measured, string_width};

/// Node labels wrap to at most this many display columns per line...
pub const WRAP_WIDTH: usize = 24;
/// ...and at most this many lines; overflow is truncated with an ellipsis.
pub const MAX_LINES: usize = 4;
/// Edge labels are truncated to this many columns.
pub const MAX_LABEL: usize = 28;

/// Identifier-boundary characters preferred as break points when a single word
/// is too wide to fit, so it is not sliced mid-segment.
const LABEL_BREAK_CHARS: [char; 4] = ['_', '-', '.', '/'];

/// ASCII-only case folding.
///
/// `char::to_lowercase` can change a string's length (`İ` becomes two code
/// points), which would desync the byte offsets some parsers slice with, so
/// only the plain ASCII letters are folded.
pub fn ascii_lower(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c })
        .collect()
}

pub fn ascii_upper(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c })
        .collect()
}

/// C0 and C1 controls, less the `\t\n\r` the parsers and `src_lines` read.
///
/// They measure one column and paint none, so a box sized around one is drawn a
/// column short of its own border; NUL also collides with the canvas `CONT`
/// sentinel and is dropped after layout has already paid for its cell; ESC
/// would inject ANSI into the caller's scrollback. `decode_entity_body` refuses
/// to decode an entity into one — this closes the same hole for literals.
fn is_control_to_strip(c: char) -> bool {
    matches!(c as u32, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f | 0x7f..=0x9f)
}

/// Applied by every public entry point that takes untrusted source.
pub fn strip_controls(src: &str) -> String {
    src.chars().filter(|&c| !is_control_to_strip(c)).collect()
}

/// Split source into lines the way `str::lines()` does: on `\n`, with a
/// trailing `\r` stripped, and *without* a final empty line when the input ends
/// in a newline. A plain split on `\n` yields that extra element, which would
/// show up as a spurious blank row inside a source box.
pub fn src_lines(src: &str) -> Vec<String> {
    let mut out: Vec<String> = src
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect();
    if out.last().is_some_and(|s| s.is_empty()) {
        out.pop();
    }
    out
}

/// Characters allowed in a bare node/state/class identifier.
pub fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

const ENTITY_LOOKAHEAD: usize = 10;

fn named_entity(body: &str) -> Option<char> {
    match body {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

fn decode_entity_body(body: &str) -> Option<String> {
    if let Some(c) = named_entity(body) {
        return Some(c.to_string());
    }
    let num = body.strip_prefix('#')?;
    let (hex, digits) = match num.strip_prefix(['x', 'X']) {
        Some(d) => (true, d),
        None => (false, num),
    };
    if digits.is_empty() {
        return None;
    }
    let valid = if hex {
        digits.chars().all(|c| c.is_ascii_hexdigit())
    } else {
        digits.chars().all(|c| c.is_ascii_digit())
    };
    if !valid {
        return None;
    }
    let code = u32::from_str_radix(digits, if hex { 16 } else { 10 }).ok()?;
    // Surrogates and out-of-range values are not characters at all.
    if code > 0x10ffff || (0xd800..=0xdfff).contains(&code) {
        return None;
    }
    // Reject control chars: NUL collides with the CONT sentinel and ESC would
    // inject ANSI into scrollback.
    if code < 0x20 || (0x7f..=0x9f).contains(&code) {
        return None;
    }
    char::from_u32(code).map(|c| c.to_string())
}

/// Decode HTML entities in label text. Called once per label: via `clean_label`
/// for bracketed labels, or explicitly at each direct-push sink.
pub fn decode_html_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '&' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // Scan a bounded window including the terminating `;`, so a stray `&` or an
        // over-long run stays literal.
        let hi = (i + 1 + ENTITY_LOOKAHEAD).min(chars.len());
        let semi = chars[i + 1..hi].iter().position(|&c| c == ';').map(|p| i + 1 + p);
        let decoded = semi.and_then(|j| {
            let body: String = chars[i + 1..j].iter().collect();
            decode_entity_body(&body)
        });
        match decoded {
            None => {
                out.push('&');
                i += 1;
            }
            Some(d) => {
                // Resume past the `;`. The single pass never re-scans emitted text, so
                // `&amp;lt;` decodes to the literal `&lt;` rather than to `<`.
                out.push_str(&d);
                i = semi.unwrap() + 1;
            }
        }
    }
    out
}

/// Strip markdown emphasis from a `` `backtick` `` label string.
pub fn strip_markdown(s: &str) -> String {
    let no_code: String = s.chars().filter(|&c| c != '`').collect();
    let no_strong = no_code.replace("**", "").replace("__", "");
    let chars: Vec<char> = no_strong.chars().collect();
    let mut out = String::new();
    for i in 0..chars.len() {
        let c = chars[i];
        // Keep `*`/`_` only when they sit inside a word, so snake_case survives.
        let in_word = i > 0
            && chars[i - 1].is_alphanumeric()
            && i + 1 < chars.len()
            && chars[i + 1].is_alphanumeric();
        if (c == '*' || c == '_') && !in_word {
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// Inline formatting tags that carry no meaning in a terminal. Anything else
/// that looks like a tag — `Vec<String>`, `<id>` — is left alone.
const HTML_FORMAT_TAGS: &[&str] = &[
    "b", "strong", "i", "em", "u", "s", "strike", "del", "ins", "mark", "small", "big", "sub",
    "sup", "code", "kbd", "samp", "var", "tt", "span", "font", "q", "abbr", "cite", "pre",
];

struct HtmlTag {
    name: String,
    end: usize,
}

/// Read a tag starting at `start`, returning its name and the index after `>`.
fn html_tag_at(chars: &[char], start: usize) -> Option<HtmlTag> {
    let mut i = start + 1;
    if chars.get(i) == Some(&'/') {
        i += 1;
    }
    let name_start = i;
    while i < chars.len() && chars[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let name: String = chars[name_start..i].iter().collect();
    while i < chars.len() && chars[i] != '>' {
        if chars[i] == '<' {
            return None;
        }
        i += 1;
    }
    if chars.get(i) == Some(&'>') {
        Some(HtmlTag { name, end: i + 1 })
    } else {
        None
    }
}

pub fn strip_html_tags(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            if let Some(tag) = html_tag_at(&chars, i) {
                let lower = tag.name.to_lowercase();
                if lower == "br" {
                    out.push(' ');
                    i = tag.end;
                    continue;
                }
                if HTML_FORMAT_TAGS.contains(&lower.as_str()) {
                    i = tag.end;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Strip one matching pair of wrapping delimiters, if present.
fn unwrap<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    if s.len() >= open.len() + close.len() && s.starts_with(open) && s.ends_with(close) {
        Some(&s[open.len()..s.len() - close.len()])
    } else {
        None
    }
}

/// Normalise raw label text: strip markup, unquote, and decode entities.
///
/// Decoding happens after tag-stripping so `<b>` is removed as markup while
/// `&lt;b&gt;` survives as the literal text `<b>`.
pub fn clean_label(raw: &str) -> String {
    let trimmed = strip_html_tags(raw.trim());
    let trimmed = trimmed.trim();
    let unquoted = unwrap(trimmed, "\"", "\"")
        .or_else(|| unwrap(trimmed, "'", "'"))
        .unwrap_or(trimmed)
        .trim();
    let md = unwrap(unquoted, "`", "`");
    let body = match md {
        Some(m) => strip_markdown(m.trim()),
        None => unquoted.to_string(),
    };
    decode_html_entities(&body)
}

/// Byte index of the last identifier-boundary character, or `None`.
fn last_break(s: &str) -> Option<usize> {
    LABEL_BREAK_CHARS.iter().filter_map(|&c| s.rfind(c)).max()
}

/// Wrap a label to `width` columns over at most `max_lines` lines, truncating
/// the last line with an ellipsis if it overflows.
///
/// A word too wide to fit is broken after the last identifier boundary
/// (`_-./`) that fits, falling back to a per-character break when it has none.
pub fn wrap_label(label: &str, width: usize, max_lines: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;

    for word in label.split_whitespace() {
        let ww = string_width(word);
        if ww > width {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            let mut chunk = String::new();
            let mut chunk_w = 0usize;
            for (ch, cw) in measured(word) {
                if chunk_w + cw > width && !chunk.is_empty() {
                    let p = last_break(&chunk);
                    let carry = match p {
                        Some(p) => chunk[p + 1..].to_string(),
                        None => String::new(),
                    };
                    lines.push(match p {
                        Some(p) => chunk[..p + 1].to_string(),
                        None => chunk.clone(),
                    });
                    chunk_w = string_width(&carry);
                    chunk = carry;
                }
                chunk.push_str(ch);
                chunk_w += cw;
            }
            cur = chunk;
            cur_w = chunk_w;
        } else if cur.is_empty() {
            cur = word.to_string();
            cur_w = ww;
        } else if cur_w + 1 + ww <= width {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + ww;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
            cur_w = ww;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }

    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last() {
            let target = width.saturating_sub(1).max(1);
            let mut s = String::new();
            let mut sw = 0usize;
            for (ch, cw) in measured(last) {
                if sw + cw > target {
                    break;
                }
                s.push_str(ch);
                sw += cw;
            }
            s.push('…');
            let idx = lines.len() - 1;
            lines[idx] = s;
        }
    }
    lines
}

/// Mermaid writes generics as `List~T~`; show them as `List<T>`.
pub(crate) fn display_generics(s: &str) -> String {
    let mut out = String::new();
    let mut open = false;
    for c in s.chars() {
        if c == '~' {
            out.push(if open { '>' } else { '<' });
            open = !open;
        } else {
            out.push(c);
        }
    }
    out
}

/// Truncate to `inner` columns, leaving room for the ellipsis.
pub fn fit_label(label: &str, inner: usize) -> String {
    if string_width(label) <= inner {
        return label.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for (c, cw) in measured(label) {
        if used + cw + 1 > inner {
            break;
        }
        out.push_str(c);
        used += cw;
    }
    out.push('…');
    out
}
