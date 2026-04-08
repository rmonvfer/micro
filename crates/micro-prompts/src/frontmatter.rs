//! The YAML-ish header at the top of a Markdown file.
//!
//! Only the shape skills actually use is understood: a block fenced by `---`, holding flat
//! `key: value` pairs. That is deliberate rather than lazy — a full YAML parser is a large
//! dependency for a header that never nests, and a file whose header this cannot read is
//! reported rather than half-understood.

/// A parsed header and the body that followed it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frontmatter {
    fields: Vec<(String, String)>,
    body: String,
}

impl Frontmatter {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Everything after the header, which is the skill itself.
    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// Read the header off a document. A document without one parses as all body.
pub fn parse_frontmatter(text: &str) -> Frontmatter {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = strip_fence(text) else {
        return Frontmatter {
            fields: Vec::new(),
            body: text.to_string(),
        };
    };

    let mut fields = Vec::new();
    let mut body = String::new();
    let mut in_header = true;

    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if in_header {
            if trimmed.trim_end() == "---" {
                in_header = false;
                continue;
            }
            if let Some((key, value)) = field(trimmed) {
                fields.push((key, value));
            }
            continue;
        }
        body.push_str(line);
    }

    // A header that was opened and never closed is not a header; the whole file is body.
    match in_header {
        true => Frontmatter {
            fields: Vec::new(),
            body: text.to_string(),
        },
        false => Frontmatter { fields, body },
    }
}

/// The text after an opening `---` line, if the document starts with one.
fn strip_fence(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---")?;
    let rest = rest.strip_prefix('\r').unwrap_or(rest);
    rest.strip_prefix('\n')
}

/// One `key: value` pair, with quotes taken off a quoted value.
fn field(line: &str) -> Option<(String, String)> {
    if line.trim_start().starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once(':')?;
    let key = key.trim();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(value);
    Some((key.to_string(), value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_is_read_and_the_body_kept() {
        let parsed =
            parse_frontmatter("---\nname: thing\ndescription: Does it.\n---\nBody here.\n");
        assert_eq!(parsed.field("name"), Some("thing"));
        assert_eq!(parsed.field("description"), Some("Does it."));
        assert_eq!(parsed.body(), "Body here.\n");
    }

    #[test]
    fn a_document_without_a_header_is_all_body() {
        let parsed = parse_frontmatter("# Just markdown\n");
        assert!(parsed.is_empty());
        assert_eq!(parsed.body(), "# Just markdown\n");
    }

    /// An unterminated header is not a header, or half a document would vanish into it.
    #[test]
    fn an_unclosed_header_leaves_the_document_alone() {
        let text = "---\nname: thing\nstill going\n";
        let parsed = parse_frontmatter(text);
        assert!(parsed.is_empty());
        assert_eq!(parsed.body(), text);
    }

    #[test]
    fn quotes_are_taken_off_a_value() {
        let parsed = parse_frontmatter("---\nname: \"quoted\"\nother: 'single'\n---\n");
        assert_eq!(parsed.field("name"), Some("quoted"));
        assert_eq!(parsed.field("other"), Some("single"));
    }

    #[test]
    fn a_value_may_hold_a_colon() {
        let parsed = parse_frontmatter("---\ndescription: Use when: it applies.\n---\n");
        assert_eq!(parsed.field("description"), Some("Use when: it applies."));
    }

    #[test]
    fn comments_and_junk_lines_are_skipped() {
        let parsed = parse_frontmatter("---\n# a comment\nname: kept\nnot a field\n---\n");
        assert_eq!(parsed.field("name"), Some("kept"));
        assert_eq!(parsed.field("not a field"), None);
    }

    #[test]
    fn windows_line_endings_are_understood() {
        let parsed = parse_frontmatter("---\r\nname: thing\r\n---\r\nbody\r\n");
        assert_eq!(parsed.field("name"), Some("thing"));
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_header() {
        let parsed = parse_frontmatter("\u{feff}---\nname: thing\n---\n");
        assert_eq!(parsed.field("name"), Some("thing"));
    }
}
